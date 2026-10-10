//! The backend: the trait's calls recorded in the bookkeeping, applied to the solver at the
//! start of each step in key order, the step, and what it reports.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use lactose_core::{Error, ErrorKind, Result, StableKey};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    BodyDesc, BodyHandle, BodyState, Collider, CollisionGroups, ContactEvent, ContactPhase, Joint,
    JointHandle, JointKind, PhysicsBackend, PhysicsCommand, QueryFilter, Ray, RayHit, RigidBody,
    Shape, ShapeHit, StepInput, StepOutput, Transform, TriggerEvent, Velocity,
};
use rapier3d_f64::dynamics::{
    FixedJointBuilder, GenericJoint, JointAxis, LockedAxes, MassProperties, PrismaticJointBuilder,
    RevoluteJointBuilder, RigidBodyActivation, RigidBodyBuilder, RigidBodySet, RigidBodyType,
    SphericalJointBuilder,
};
use rapier3d_f64::geometry::{
    ActiveCollisionTypes, ColliderBuilder, ColliderHandle, ColliderSet, CollisionEvent,
    ContactPair, Group, InteractionGroups, InteractionTestMode,
};
use rapier3d_f64::math::{Pose, Vector};
use rapier3d_f64::pipeline::{ActiveEvents, EventHandler, PhysicsWorld};
use rayon::ThreadPool;

use crate::book::{BodyEntry, Book, Desc, JointEntry, Key, Live, MassPlan, Plain};
use crate::convert::{
    key_code, key_from_code, pose, rotation, same, same_pose, same_vector, shared_shape, transform,
    valid_body, valid_collider, valid_joint, valid_transform, valid_velocity, vec3, vector,
};
use crate::query;

/// The log target the `PhysicsBackend` contract names: refused values, unknown keys and
/// dropped commands.
const CONTRACT_TARGET: &str = "physics";

/// The log target of what only this backend logs.
pub(crate) const BACKEND_TARGET: &str = "quark";

/// The seeds of the handle values: a handle is a hash of the key, so it does not depend on
/// the order bodies were created in.
const BODY_HANDLE_SEED: u64 = 0x6c70_7262_6f64_7931;
const JOINT_HANDLE_SEED: u64 = 0x6c70_726a_6f69_6e74;

pub(crate) struct Rapier {
    pub(crate) world: PhysicsWorld,
    pub(crate) pool: Arc<ThreadPool>,
    pub(crate) book: Book,
}

impl Rapier {
    pub(crate) fn new(pool: Arc<ThreadPool>) -> Self {
        let mut world = PhysicsWorld::new();
        world.set_thread_pool(Some(Arc::clone(&pool)));
        Self { world, pool, book: Book::default() }
    }
}

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

fn handle_value(key: StableKey, seed: u64) -> (u32, u32) {
    let hash = lactose_core::hash::xxh64(&key_code(key).to_le_bytes(), seed);
    (hash as u32, (hash >> 32) as u32)
}

/// What the step placed and swept, for its report.
#[derive(Default)]
struct Moved {
    /// Placed without a sweep: reported with `teleported`.
    placed: BTreeSet<Key>,
    /// Given a pose to sweep to: reported whatever the solver says of its activity.
    swept: BTreeSet<Key>,
}

impl PhysicsBackend for Rapier {
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle> {
        if self.book.bodies.contains_key(&Key(key)) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "create_body for a key the backend holds");
            return Err(invalid("create_body for a key the backend already holds"));
        }
        if desc.body.is_none() && desc.collider.is_none() {
            return Err(invalid("a body needs a RigidBody, a Collider or both"));
        }
        if !valid_transform(desc.transform) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "refused body: not a valid transform");
            return Err(invalid("not a valid transform"));
        }
        if desc.body.is_some_and(|body| !valid_body(&body)) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "refused body: not a valid RigidBody");
            return Err(invalid("not a valid RigidBody"));
        }
        if desc.collider.is_some_and(|collider| !valid_collider(collider)) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "refused body: not a valid Collider");
            return Err(invalid("not a valid Collider"));
        }
        let velocity = desc.velocity.copied().filter(valid_velocity);
        if desc.velocity.is_some() && velocity.is_none() {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "refused body: not a valid Velocity");
            return Err(invalid("not a valid Velocity"));
        }
        let entry = BodyEntry {
            wanted: Desc {
                body: desc.body.map(Plain),
                collider: desc.collider.copied().map(Plain),
                frozen: desc.frozen,
            },
            live: None,
            transform: Some(Plain(*desc.transform)),
            velocity: velocity.map(Plain),
            parked: None,
        };
        self.book.bodies.insert(Key(key), entry);
        self.book.dirty.insert(Key(key));
        let (index, generation) = handle_value(key, BODY_HANDLE_SEED);
        Ok(BodyHandle::new(index, generation))
    }

    fn remove_body(&mut self, key: StableKey) {
        let Some(entry) = self.book.bodies.remove(&Key(key)) else {
            tracing::debug!(target: CONTRACT_TARGET, ?key, "remove_body for a key the backend does not hold");
            return;
        };
        self.book.dirty.remove(&Key(key));
        if let Some(live) = entry.live {
            // The solver removes the body's collider with it.
            self.book.doomed_bodies.insert(Key(key), live);
        }
        let doomed = &mut self.book.doomed_joints;
        self.book.joints.retain(|&joint_key, joint| {
            let kept = joint.body_a != Key(key) && joint.body_b != Key(key);
            if !kept && let Some(handle) = joint.handle {
                doomed.insert(joint_key, handle);
            }
            kept
        });
    }

    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>) {
        if body.is_some_and(|body| !valid_body(body)) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "ignored set_body: not a valid RigidBody");
            return;
        }
        if let Some(entry) = self.wanted(key) {
            let body = body.copied().map(Plain);
            if entry.wanted.body != body {
                entry.hold();
                entry.wanted.body = body;
                self.book.dirty.insert(Key(key));
            }
        }
    }

    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>) {
        if collider.is_some_and(|collider| !valid_collider(collider)) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "ignored set_collider: not a valid Collider");
            return;
        }
        if let Some(entry) = self.wanted(key) {
            let collider = collider.copied().map(Plain);
            if entry.wanted.collider != collider {
                entry.hold();
                entry.wanted.collider = collider;
                self.book.dirty.insert(Key(key));
            }
        }
    }

    fn set_transform(&mut self, key: StableKey, transform: &Transform) {
        if !valid_transform(transform) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "ignored set_transform: not a valid transform");
            return;
        }
        if let Some(entry) = self.wanted(key) {
            entry.transform = Some(Plain(*transform));
            self.book.dirty.insert(Key(key));
        }
    }

    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity) {
        if !valid_velocity(velocity) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "ignored set_velocity: not a valid Velocity");
            return;
        }
        if let Some(entry) = self.wanted(key) {
            entry.velocity = Some(Plain(*velocity));
            self.book.dirty.insert(Key(key));
        }
    }

    fn set_frozen(&mut self, key: StableKey, frozen: bool) {
        if let Some(entry) = self.wanted(key)
            && entry.wanted.frozen != frozen
        {
            entry.hold();
            entry.wanted.frozen = frozen;
            self.book.dirty.insert(Key(key));
        }
    }

    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle> {
        if self.book.joints.contains_key(&Key(key)) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "create_joint for a key the backend holds");
            return Err(invalid("create_joint for a key the backend already holds"));
        }
        if body_a == body_b {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "refused joint: both ends are one body");
            return Err(invalid("a joint between a body and itself"));
        }
        if !valid_joint(joint) {
            tracing::warn!(target: CONTRACT_TARGET, ?key, "refused joint: not a valid Joint");
            return Err(invalid("not a valid Joint"));
        }
        for end in [body_a, body_b] {
            if !self.book.bodies.contains_key(&Key(end)) {
                tracing::warn!(target: CONTRACT_TARGET, ?key, body = ?end, "refused joint: no such body");
                return Err(Error::new(ErrorKind::NotFound, "a joint's body is not held"));
            }
        }
        self.book.joints.insert(
            Key(key),
            JointEntry {
                joint: Plain(*joint),
                body_a: Key(body_a),
                body_b: Key(body_b),
                handle: None,
            },
        );
        let (index, generation) = handle_value(key, JOINT_HANDLE_SEED);
        Ok(JointHandle::new(index, generation))
    }

    fn remove_joint(&mut self, key: StableKey) {
        let Some(entry) = self.book.joints.remove(&Key(key)) else {
            tracing::debug!(target: CONTRACT_TARGET, ?key, "remove_joint for a key the backend does not hold");
            return;
        };
        if let Some(handle) = entry.handle {
            self.book.doomed_joints.insert(Key(key), handle);
        }
    }

    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        // A step of zero length returns before the solver joins its deferred broad-phase
        // work, which would leave the tree out of the world.
        if !input.dt.is_finite() || input.dt <= Scalar::ZERO || !input.gravity.is_finite() {
            tracing::warn!(target: CONTRACT_TARGET, tick = input.tick.0, "skipped step: not a valid dt or gravity");
            return;
        }
        let mut moved = Moved::default();
        let removed = self.flush(&mut moved);
        let forced = self.apply_commands(input.commands, &mut moved);
        if !same_vector(self.world.gravity, vector(input.gravity)) {
            self.world.gravity = vector(input.gravity);
        }
        self.world.integration_parameters.dt = input.dt.to_f64();

        let mut candidates = self.moving();
        let collector = Collector { removed: &removed, events: Mutex::new(Vec::new()) };
        self.world.step_with_events(&(), &collector);
        let events = collector.events.into_inner().unwrap_or_else(PoisonError::into_inner);

        for handle in forced {
            let unforced = self.world.bodies.get(handle).is_none_or(|body| {
                same_vector(body.user_force(), Vector::ZERO)
                    && same_vector(body.user_torque(), Vector::ZERO)
            });
            if !unforced && let Some(body) = self.world.bodies.get_mut(handle) {
                body.reset_forces(false);
                body.reset_torques(false);
            }
        }

        // Moving before the step or after it: a body that came to rest in this step is
        // reported once more, with its zero velocity.
        candidates.extend(self.moving());
        let mut reported: Vec<Key> = candidates
            .into_iter()
            .chain(moved.placed.iter().copied())
            .chain(moved.swept.iter().copied())
            .collect();
        reported.sort_unstable();
        reported.dedup();
        for key in reported {
            let Some(live) = self.book.bodies.get(&key).and_then(|entry| entry.live) else {
                continue;
            };
            let Some(body) = self.world.bodies.get(live.body) else { continue };
            output.bodies.push(BodyState {
                key: key.0,
                transform: transform(body.position()),
                velocity: Velocity { linear: vec3(body.linvel()), angular: vec3(body.angvel()) },
                teleported: moved.placed.contains(&key),
            });
        }
        self.report_events(events, output);
    }

    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()> {
        crate::snapshot::write(self, out)
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        let (world, book) = crate::snapshot::read(bytes, &self.pool)?;
        self.world = world;
        self.book = book;
        Ok(())
    }

    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        query::cast_ray(self, ray, filter)
    }

    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        query::cast_shape(self, shape, pose, direction, max_distance, filter)
    }

    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        query::overlap(self, shape, pose, filter, hits);
    }
}

// ---- Applying the bookkeeping ----

impl Rapier {
    /// The keys of the awake dynamic bodies and of the kinematic bodies with a velocity:
    /// what the step reports, with the bodies it placed or swept.
    fn moving(&self) -> Vec<Key> {
        self.world
            .islands
            .active_bodies()
            .filter_map(|handle| self.world.bodies.get(handle))
            .filter(|body| {
                body.is_dynamic()
                    || (body.is_kinematic()
                        && (vec3(body.linvel()) != Vec3::ZERO || vec3(body.angvel()) != Vec3::ZERO))
            })
            .filter_map(|body| key_from_code(body.user_data).map(Key))
            .collect()
    }

    /// The entry of `key`, or `None` and a log line.
    fn wanted(&mut self, key: StableKey) -> Option<&mut BodyEntry> {
        let entry = self.book.bodies.get_mut(&Key(key));
        if entry.is_none() {
            tracing::debug!(target: CONTRACT_TARGET, ?key, "a call for a key the backend does not hold");
        }
        entry
    }

    /// Applies the bookkeeping's waiting work to the solver: joint removals, body removals,
    /// then every dirty body, then the waiting joints, each in ascending key order. Returns
    /// the colliders it removed, by handle, so the step's events can name them.
    fn flush(&mut self, moved: &mut Moved) -> BTreeMap<(u32, u32), StableKey> {
        for (_, handle) in std::mem::take(&mut self.book.doomed_joints) {
            self.world.impulse_joints.remove(handle, true);
        }
        let mut removed = BTreeMap::new();
        for (key, live) in std::mem::take(&mut self.book.doomed_bodies) {
            if let Some(collider) = live.collider {
                removed.insert(collider.into_raw_parts(), key.0);
            }
            self.world.remove_body(live.body);
        }
        for key in std::mem::take(&mut self.book.dirty) {
            let Some(mut entry) = self.book.bodies.remove(&key) else { continue };
            match entry.live {
                None => self.insert(key, &mut entry),
                Some(live) => self.update(key, live, &mut entry, moved, &mut removed),
            }
            self.book.bodies.insert(key, entry);
        }
        for (&key, joint) in &mut self.book.joints {
            if joint.handle.is_some() {
                continue;
            }
            let end = |end: Key| self.book.bodies.get(&end).and_then(|entry| entry.live);
            let (Some(a), Some(b)) = (end(joint.body_a), end(joint.body_b)) else {
                tracing::warn!(target: BACKEND_TARGET, key = ?key.0, "a joint whose bodies are not in the solver");
                continue;
            };
            let data = generic_joint(&joint.joint.0, key_code(key.0));
            joint.handle = Some(self.world.impulse_joints.insert(a.body, b.body, data, true));
        }
        removed
    }

    /// Puts a body the solver does not hold yet into it.
    fn insert(&mut self, key: Key, entry: &mut BodyEntry) {
        let wanted = entry.wanted;
        let Some(Plain(at)) = entry.transform.take() else {
            // The snapshot reader refuses bytes that break this, and the calls cannot.
            unreachable!("a body waiting for the solver has the transform it was created with");
        };
        let code = key_code(key.0);
        let body_type = wanted.body_type();
        let mut builder = RigidBodyBuilder::new(body_type).pose(pose(&at)).user_data(code);
        if let Some(Plain(body)) = wanted.body {
            builder = builder
                .linear_damping(body.linear_damping.to_f64())
                .angular_damping(body.angular_damping.to_f64())
                .gravity_scale(body.gravity_scale.to_f64())
                .locked_axes(LockedAxes::from_bits_truncate(body.locked_axes))
                .can_sleep(body.can_sleep);
        }
        if let MassPlan::Additional(mass) = wanted.mass_plan() {
            builder = builder.additional_mass_properties(unit_ball(mass));
        }
        if let Some(Plain(velocity)) = entry.velocity.take() {
            if body_type == RigidBodyType::Dynamic {
                builder = builder.linvel(vector(velocity.linear)).angvel(vector(velocity.angular));
            } else if parks(&wanted) {
                entry.parked = Some(Plain(velocity));
            }
        }
        let body = self.world.insert_body(builder);
        let collider = wanted.collider.map(|Plain(collider)| {
            let built = build_collider(code, &collider, wanted.mass_plan());
            self.world.insert_collider(built, Some(body))
        });
        self.recompute_mass(body);
        entry.live = Some(Live { held: None, body, collider });
    }

    /// Brings a body the solver holds to what the rows say, touching only what differs.
    fn update(
        &mut self,
        key: Key,
        live: Live,
        entry: &mut BodyEntry,
        moved: &mut Moved,
        removed: &mut BTreeMap<(u32, u32), StableKey>,
    ) {
        let (held, wanted) = (live.held.unwrap_or(entry.wanted), entry.wanted);
        let code = key_code(key.0);
        let handle = live.body;
        let mut collider = live.collider;
        let mut wake = false;

        // The collider: added, removed or changed in place.
        let mut built = false;
        match (held.collider, wanted.collider, collider) {
            (Some(_), None, Some(old)) => {
                self.world.remove_collider(old);
                removed.insert(old.into_raw_parts(), key.0);
                collider = None;
                wake = true;
            }
            (None, Some(Plain(new)), _) => {
                let made = build_collider(code, &new, wanted.mass_plan());
                collider = Some(self.world.insert_collider(made, Some(handle)));
                built = true;
                wake = true;
            }
            (Some(Plain(old)), Some(Plain(new)), Some(handle)) if !same(&old, &new) => {
                if let Some(solver) = self.world.colliders.get_mut(handle) {
                    reconcile_collider(&old, &new, solver);
                }
                wake = true;
            }
            _ => {}
        }

        // The type, parking a frozen body's velocity and giving it back on thawing.
        let (old_type, new_type) = (held.body_type(), wanted.body_type());
        if old_type != new_type
            && let Some(body) = self.world.bodies.get_mut(handle)
        {
            if parks(&wanted) && !held.frozen {
                entry.parked = Some(Plain(Velocity {
                    linear: vec3(body.linvel()),
                    angular: vec3(body.angvel()),
                }));
            }
            body.set_body_type(new_type, true);
            if new_type == RigidBodyType::Dynamic
                && let Some(Plain(parked)) = entry.parked.take()
            {
                body.set_linvel(vector(parked.linear), true);
                body.set_angvel(vector(parked.angular), true);
            }
            wake = true;
        }
        if !wanted.frozen && new_type != RigidBodyType::Dynamic {
            entry.parked = None;
        }

        // The body's own values.
        if let Some(Plain(new)) = wanted.body {
            let old = held.body.map(|Plain(old)| old);
            if old.is_none_or(|old| !same(&old, &new))
                && let Some(body) = self.world.bodies.get_mut(handle)
            {
                reconcile_body(old.as_ref(), &new, body);
                wake = true;
            }
        }

        // Where the mass comes from.
        let (old_plan, new_plan) = (held.mass_plan(), wanted.mass_plan());
        if old_plan != new_plan {
            if let MassPlan::Additional(_) = old_plan
                && let Some(body) = self.world.bodies.get_mut(handle)
            {
                body.set_additional_mass_properties(MassProperties::default(), true);
            }
            match new_plan {
                MassPlan::Additional(mass) => {
                    if let Some(body) = self.world.bodies.get_mut(handle) {
                        body.set_additional_mass_properties(unit_ball(mass), true);
                    }
                }
                MassPlan::ColliderMass(mass) if !built => {
                    if let Some(solver) = collider.and_then(|h| self.world.colliders.get_mut(h)) {
                        solver.set_mass(mass);
                    }
                }
                MassPlan::ColliderDensity(density) if !built => {
                    if let Some(solver) = collider.and_then(|h| self.world.colliders.get_mut(h)) {
                        solver.set_density(density);
                    }
                }
                MassPlan::ColliderMass(_) | MassPlan::ColliderDensity(_) => {}
            }
            wake = true;
        }

        // A transform write: a sweep for a kinematic body, a placement for any other.
        if let Some(Plain(at)) = entry.transform.take() {
            let target = pose(&at);
            if new_type == RigidBodyType::KinematicPositionBased {
                let differs = self
                    .world
                    .bodies
                    .get(handle)
                    .is_some_and(|b| !same_pose(b.next_position(), &target));
                if differs && let Some(body) = self.world.bodies.get_mut(handle) {
                    body.set_next_kinematic_position(target);
                    moved.swept.insert(key);
                    wake = true;
                }
            } else {
                let differs = self
                    .world
                    .bodies
                    .get(handle)
                    .is_some_and(|b| !same_pose(b.position(), &target));
                if differs && let Some(body) = self.world.bodies.get_mut(handle) {
                    body.set_position(target, true);
                    moved.placed.insert(key);
                    moved.swept.remove(&key);
                    wake = true;
                }
            }
        }

        // A velocity write: ignored for a kinematic or fixed body, kept for a frozen one.
        if let Some(Plain(velocity)) = entry.velocity.take() {
            if new_type == RigidBodyType::Dynamic {
                let (linear, angular) = (vector(velocity.linear), vector(velocity.angular));
                let differs = self.world.bodies.get(handle).is_some_and(|b| {
                    !same_vector(b.linvel(), linear) || !same_vector(b.angvel(), angular)
                });
                if differs && let Some(body) = self.world.bodies.get_mut(handle) {
                    body.set_linvel(linear, true);
                    body.set_angvel(angular, true);
                    wake = true;
                }
            } else if parks(&wanted) {
                entry.parked = Some(Plain(velocity));
            }
        }

        if wake
            && new_type != RigidBodyType::Fixed
            && let Some(body) = self.world.bodies.get_mut(handle)
        {
            body.wake_up(true);
        }
        if wake {
            self.recompute_mass(handle);
        }
        entry.live = Some(Live { held: None, body: handle, collider });
    }

    /// Brings the body's mass properties to its colliders and additional mass now. Rapier
    /// otherwise does it in the step's user-change pass, after the tick's commands, so an
    /// impulse in the tick of a creation or a mass change would see the old mass.
    fn recompute_mass(&mut self, handle: rapier3d_f64::dynamics::RigidBodyHandle) {
        let PhysicsWorld { bodies, colliders, .. } = &mut self.world;
        if let Some(body) = bodies.get_mut(handle) {
            body.recompute_mass_properties_from_colliders(colliders);
        }
    }

    /// Applies the tick's commands in order. Returns the bodies given a force or torque,
    /// whose accumulators the step clears afterwards.
    fn apply_commands(
        &mut self,
        commands: &[PhysicsCommand],
        moved: &mut Moved,
    ) -> Vec<rapier3d_f64::dynamics::RigidBodyHandle> {
        let mut forced = Vec::new();
        for command in commands {
            let key = Key(command.body());
            let Some((live, wanted)) =
                self.book.bodies.get(&key).and_then(|entry| Some((entry.live?, entry.wanted)))
            else {
                tracing::debug!(target: CONTRACT_TARGET, key = ?key.0, "dropped command: no such body");
                continue;
            };
            // Every test before the mutable access, which marks the body modified. After the
            // flush, what the solver holds is what the entry wants.
            let honoured = match *command {
                PhysicsCommand::Teleport { transform, .. } => valid_transform(&transform),
                _ if wanted.body_type() != RigidBodyType::Dynamic => {
                    tracing::debug!(target: CONTRACT_TARGET, key = ?key.0, "dropped command: the body is not dynamic");
                    continue;
                }
                PhysicsCommand::ApplyImpulseAt { impulse, point, .. } => {
                    impulse.is_finite() && point.is_finite()
                }
                PhysicsCommand::ApplyImpulse { impulse: v, .. }
                | PhysicsCommand::ApplyTorqueImpulse { torque: v, .. }
                | PhysicsCommand::ApplyForce { force: v, .. }
                | PhysicsCommand::ApplyTorque { torque: v, .. } => v.is_finite(),
            };
            if !honoured {
                tracing::warn!(target: CONTRACT_TARGET, key = ?key.0, "dropped command: not a valid value");
                continue;
            }
            let Some(body) = self.world.bodies.get_mut(live.body) else { continue };
            match *command {
                PhysicsCommand::Teleport { transform, .. } => {
                    body.set_position(pose(&transform), true);
                    moved.placed.insert(key);
                    moved.swept.remove(&key);
                }
                PhysicsCommand::ApplyImpulse { impulse, .. } => {
                    body.apply_impulse(vector(impulse), true);
                }
                PhysicsCommand::ApplyImpulseAt { impulse, point, .. } => {
                    body.apply_impulse_at_point(vector(impulse), vector(point), true);
                }
                PhysicsCommand::ApplyTorqueImpulse { torque, .. } => {
                    body.apply_torque_impulse(vector(torque), true);
                }
                PhysicsCommand::ApplyForce { force, .. } => {
                    body.add_force(vector(force), true);
                    forced.push(live.body);
                }
                PhysicsCommand::ApplyTorque { torque, .. } => {
                    body.add_torque(vector(torque), true);
                    forced.push(live.body);
                }
            }
        }
        forced
    }

    /// Turns the solver's events into the trait's: `Started` and `Stopped` to both sides,
    /// `Impact` to each side whose own collider has `events` and whose threshold the pair's
    /// impulse exceeds, triggers to both sides, each at most once per (`target`, `other`,
    /// `phase`).
    fn report_events(&self, events: Vec<Raw>, output: &mut StepOutput) {
        for raw in events {
            match raw {
                Raw::Contact { a, b, phase, point, normal, impulse } => {
                    let (point, impulse) = (vec3(point), Scalar::from_f64(impulse));
                    let normal = vec3(normal);
                    output.contacts.push(ContactEvent {
                        target: a,
                        other: b,
                        phase,
                        point,
                        normal: -normal,
                        impulse,
                    });
                    output.contacts.push(ContactEvent {
                        target: b,
                        other: a,
                        phase,
                        point,
                        normal,
                        impulse,
                    });
                }
                Raw::Force { a, b, point, normal, impulse } => {
                    let (point, impulse) = (vec3(point), Scalar::from_f64(impulse));
                    let normal = vec3(normal);
                    for (target, other, normal) in [(a, b, -normal), (b, a, normal)] {
                        if self.impact_threshold(target).is_some_and(|limit| impulse > limit) {
                            output.contacts.push(ContactEvent {
                                target,
                                other,
                                phase: ContactPhase::Impact,
                                point,
                                normal,
                                impulse,
                            });
                        }
                    }
                }
                Raw::Trigger { a, b, phase } => {
                    output.triggers.push(TriggerEvent { target: a, other: b, phase });
                    output.triggers.push(TriggerEvent { target: b, other: a, phase });
                }
            }
        }
        // A pair the solver reported twice keeps the copy that sorts first by value, so the
        // copy kept does not depend on the order the solver's threads reported in.
        output.contacts.sort_by_key(contact_order);
        output
            .contacts
            .dedup_by(|x, y| (x.target, x.other, x.phase) == (y.target, y.other, y.phase));
        output.triggers.sort_by_key(|event| (event.target, event.other, event.phase));
        output.triggers.dedup();
    }

    /// The impact threshold of `key`'s collider, when it has `events` and is solid. Read
    /// after the step, when the solver holds what each entry wants.
    fn impact_threshold(&self, key: StableKey) -> Option<Scalar> {
        let entry = self.book.bodies.get(&Key(key))?;
        entry.live?;
        let Plain(collider) = entry.wanted.collider?;
        (collider.events && !collider.sensor).then_some(collider.impact_threshold)
    }
}

fn contact_order(event: &ContactEvent) -> (StableKey, StableKey, ContactPhase, [u64; 7]) {
    let bits = |s: Scalar| s.to_canonical_bits();
    let [px, py, pz] = event.point.to_array().map(bits);
    let [nx, ny, nz] = event.normal.to_array().map(bits);
    (event.target, event.other, event.phase, [px, py, pz, nx, ny, nz, bits(event.impulse)])
}

/// Whether a velocity written to the body while it is fixed is kept for when it thaws: a
/// dynamic body in a frozen region.
fn parks(wanted: &Desc) -> bool {
    wanted.frozen
        && wanted
            .body
            .is_some_and(|Plain(body)| body.body_type == lactose_physics_api::BodyType::Dynamic)
}

/// A solid unit ball's mass properties: what a body without a collider weighs.
fn unit_ball(mass: f64) -> MassProperties {
    let inertia = 0.4 * mass;
    MassProperties::new(Vector::ZERO, mass, Vector::splat(inertia))
}

pub(crate) fn groups(groups: CollisionGroups) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_retain(groups.membership),
        Group::from_bits_retain(groups.filter),
        InteractionTestMode::And,
    )
}

/// What the solver reports for the collider: contacts and contact forces for a solid one
/// with `events`, overlaps for a sensor with them.
fn active_events(collider: &Collider) -> ActiveEvents {
    match (collider.events, collider.sensor) {
        (false, _) => ActiveEvents::empty(),
        (true, true) => ActiveEvents::COLLISION_EVENTS,
        (true, false) => ActiveEvents::COLLISION_EVENTS | ActiveEvents::CONTACT_FORCE_EVENTS,
    }
}

/// Which pairs the solver looks at: a sensor sees every body but another fixed one; a
/// solid collider the solver's default (pairs with at least one dynamic body).
fn collision_types(collider: &Collider) -> ActiveCollisionTypes {
    if collider.sensor {
        ActiveCollisionTypes::all() - ActiveCollisionTypes::FIXED_FIXED
    } else {
        ActiveCollisionTypes::default()
    }
}

fn collider_pose(collider: &Collider) -> Pose {
    Pose::from_parts(vector(collider.offset), rotation(collider.rotation))
}

fn build_collider(
    code: u128,
    collider: &Collider,
    plan: MassPlan,
) -> rapier3d_f64::geometry::Collider {
    // The force threshold is zero for every collider with `events`: the impact test is
    // against the impulse and the target's own threshold, made after the step.
    let builder = ColliderBuilder::new(shared_shape(&collider.shape))
        .position(collider_pose(collider))
        .friction(collider.friction.to_f64())
        .restitution(collider.restitution.to_f64())
        .collision_groups(groups(collider.groups))
        .sensor(collider.sensor)
        .active_events(active_events(collider))
        .active_collision_types(collision_types(collider))
        .contact_force_event_threshold(0.0)
        .user_data(code);
    match plan {
        MassPlan::ColliderMass(mass) => builder.mass(mass),
        MassPlan::ColliderDensity(density) | MassPlan::Additional(density) => {
            builder.density(density)
        }
    }
    .build()
}

/// Sets on the solver's collider each value of `new` that differs from `old`. The density
/// and the mass are [`MassPlan`]'s; `impact_threshold` and `material` are read only from the
/// bookkeeping.
fn reconcile_collider(
    old: &Collider,
    new: &Collider,
    solver: &mut rapier3d_f64::geometry::Collider,
) {
    if !same(&old.shape, &new.shape) {
        solver.set_shape(shared_shape(&new.shape));
    }
    if !same(&old.offset, &new.offset) || !same(&old.rotation, &new.rotation) {
        solver.set_position_wrt_parent(collider_pose(new));
    }
    if !same(&old.friction, &new.friction) {
        solver.set_friction(new.friction.to_f64());
    }
    if !same(&old.restitution, &new.restitution) {
        solver.set_restitution(new.restitution.to_f64());
    }
    if old.groups != new.groups {
        solver.set_collision_groups(groups(new.groups));
    }
    if old.sensor != new.sensor {
        solver.set_sensor(new.sensor);
        solver.set_active_collision_types(collision_types(new));
    }
    if old.sensor != new.sensor || old.events != new.events {
        solver.set_active_events(active_events(new));
    }
}

/// Sets on the solver's body each value of `new` that differs from `old` (every value when
/// the body was a fixed collider). The type is the caller's, the mass [`MassPlan`]'s.
fn reconcile_body(
    old: Option<&RigidBody>,
    new: &RigidBody,
    body: &mut rapier3d_f64::dynamics::RigidBody,
) {
    let differs =
        |pick: fn(&RigidBody) -> Scalar| old.is_none_or(|old| !same(&pick(old), &pick(new)));
    if differs(|b| b.linear_damping) {
        body.set_linear_damping(new.linear_damping.to_f64());
    }
    if differs(|b| b.angular_damping) {
        body.set_angular_damping(new.angular_damping.to_f64());
    }
    if differs(|b| b.gravity_scale) {
        body.set_gravity_scale(new.gravity_scale.to_f64(), true);
    }
    if old.is_none_or(|old| old.locked_axes != new.locked_axes) {
        body.set_locked_axes(LockedAxes::from_bits_truncate(new.locked_axes), true);
    }
    if old.is_none_or(|old| old.can_sleep != new.can_sleep) {
        let activation = body.activation_mut();
        if new.can_sleep {
            activation.normalized_linear_threshold =
                RigidBodyActivation::default_normalized_linear_threshold();
            activation.angular_threshold = RigidBodyActivation::default_angular_threshold();
        } else {
            activation.normalized_linear_threshold = -1.0;
            activation.angular_threshold = -1.0;
        }
    }
}

/// The solver's joint for a valid `joint`: anchors in each body's space, the axis in both
/// (`Joint::axis` is in the first body's; Rapier's builders put one axis in both frames),
/// and the limits on the axis a revolute or prismatic joint frees, or on all three angular
/// axes of a spherical one.
fn generic_joint(joint: &Joint, code: u128) -> GenericJoint {
    let (a, b) = (vector(joint.anchor_a), vector(joint.anchor_b));
    // Only the kinds that read the axis normalise it: `valid_joint` lets a zero one through
    // for the others.
    let axis = || vector(joint.axis.normalize());
    let limits = [joint.limit_min.to_f64(), joint.limit_max.to_f64()];
    let mut data: GenericJoint = match joint.kind {
        JointKind::Fixed => FixedJointBuilder::new().local_anchor1(a).local_anchor2(b).into(),
        JointKind::Spherical => {
            let mut builder = SphericalJointBuilder::new().local_anchor1(a).local_anchor2(b);
            if joint.limited {
                for free in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                    builder = builder.limits(free, limits);
                }
            }
            builder.into()
        }
        JointKind::Revolute => {
            let mut builder = RevoluteJointBuilder::new(axis()).local_anchor1(a).local_anchor2(b);
            if joint.limited {
                builder = builder.limits(limits);
            }
            builder.into()
        }
        JointKind::Prismatic => {
            let mut builder = PrismaticJointBuilder::new(axis()).local_anchor1(a).local_anchor2(b);
            if joint.limited {
                builder = builder.limits(limits);
            }
            builder.into()
        }
    };
    data.user_data = code;
    data
}

// ---- Events ----

/// One solver event, keyed, before it becomes the trait's.
pub(crate) enum Raw {
    Contact {
        a: StableKey,
        b: StableKey,
        phase: ContactPhase,
        point: Vector,
        normal: Vector,
        impulse: f64,
    },
    Force {
        a: StableKey,
        b: StableKey,
        point: Vector,
        normal: Vector,
        impulse: f64,
    },
    Trigger {
        a: StableKey,
        b: StableKey,
        phase: ContactPhase,
    },
}

/// Collects the step's events. The solver may call it from its threads; the order it
/// collects in is undone by [`Rapier::report_events`]' sort.
struct Collector<'a> {
    /// The colliders removed at the start of the step, which the solver reports a `Stopped`
    /// for after they are gone.
    removed: &'a BTreeMap<(u32, u32), StableKey>,
    events: Mutex<Vec<Raw>>,
}

impl Collector<'_> {
    fn key(&self, colliders: &ColliderSet, handle: ColliderHandle) -> Option<StableKey> {
        match colliders.get(handle) {
            Some(collider) => key_from_code(collider.user_data),
            None => self.removed.get(&handle.into_raw_parts()).copied(),
        }
    }

    fn push(&self, raw: Raw) {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).push(raw);
    }
}

/// The deepest contact of the pair in world space and the pair's normal, from the first
/// collider towards the second; zero when the pair has no contact.
fn deepest(colliders: &ColliderSet, pair: &ContactPair) -> (Vector, Vector) {
    let Some(collider) = colliders.get(pair.collider1) else { return (Vector::ZERO, Vector::ZERO) };
    match pair.find_deepest_contact() {
        Some((manifold, contact)) => {
            (collider.position().transform_point(contact.local_p1), manifold.data.normal)
        }
        None => (Vector::ZERO, Vector::ZERO),
    }
}

impl EventHandler for Collector<'_> {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        colliders: &ColliderSet,
        event: CollisionEvent,
        pair: Option<&ContactPair>,
    ) {
        let (Some(a), Some(b)) =
            (self.key(colliders, event.collider1()), self.key(colliders, event.collider2()))
        else {
            return;
        };
        let phase = if event.started() { ContactPhase::Started } else { ContactPhase::Stopped };
        if event.sensor() {
            self.push(Raw::Trigger { a, b, phase });
            return;
        }
        let (point, normal, impulse) = match (phase, pair) {
            (ContactPhase::Started, Some(pair)) => {
                let (point, normal) = deepest(colliders, pair);
                (point, normal, pair.total_impulse_magnitude())
            }
            _ => (Vector::ZERO, Vector::ZERO, 0.0),
        };
        self.push(Raw::Contact { a, b, phase, point, normal, impulse });
    }

    fn handle_contact_force_event(
        &self,
        _dt: f64,
        _bodies: &RigidBodySet,
        colliders: &ColliderSet,
        pair: &ContactPair,
        _total_force_magnitude: f64,
    ) {
        let (Some(a), Some(b)) =
            (self.key(colliders, pair.collider1), self.key(colliders, pair.collider2))
        else {
            return;
        };
        let (point, normal) = deepest(colliders, pair);
        self.push(Raw::Force { a, b, point, normal, impulse: pair.total_impulse_magnitude() });
    }

    fn handle_soft_body_tear_event(
        &self,
        _soft_bodies: &rapier3d_f64::dynamics::SoftBodySet,
        _event: &rapier3d_f64::dynamics::SoftBodyTearEvent,
    ) {
    }
}
