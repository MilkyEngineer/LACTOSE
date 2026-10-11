//! What the three systems do (`README.md`, "The tick"): the sync, which makes the backend's
//! bodies and joints follow the rows and takes the commands; the step, which hands the
//! backend the commands and sorts what it reports; and the write-back, which writes what
//! moved into the rows. Methods of [`Physics`], so the systems in `systems.rs` stay the
//! access tables and nothing else.

use core::mem;
use core::ops::{BitOr, BitOrAssign};
use std::collections::{BTreeMap, BTreeSet};

use lactose_core::{ComponentKey, StableKey};
use lactose_ecs::{
    ChangeMark, ClaimedBy, EventWriter, Job, LogCursor, LogEntries, RegionId, RegionMode,
    StructuralChange, World, WorldKind,
};
use lactose_math::{Canonical, Scalar};
use lactose_reflect::Reflect;

use crate::backend::{BodyDesc, BodyState, PhysicsBackend, StepInput};
use crate::command::{PhysicsCommand, PhysicsCommands};
use crate::components::{
    BodyHandle, BodyType, Collider, Joint, JointHandle, RigidBody, Teleported, Transform, Velocity,
};
use crate::event::{ContactEvent, TriggerEvent};
use crate::service::{JointEntry, Physics, SyncState};

/// Which of the five columns the sync reads a key changed in.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct Changes(u8);

impl Changes {
    const TRANSFORM: Self = Self(1);
    const VELOCITY: Self = Self(1 << 1);
    const BODY: Self = Self(1 << 2);
    const COLLIDER: Self = Self(1 << 3);
    const JOINT: Self = Self(1 << 4);
    /// Every column: a key [`corrected`](crate::corrected) names.
    pub(crate) const ALL: Self = Self(0b1_1111);

    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Changes {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for Changes {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// Adds to `keys` every row of the five columns changed since `mark`, with its columns.
/// `mark` is current.
pub(crate) fn changed_since(
    world: &World,
    mark: ChangeMark,
    keys: &mut BTreeMap<StableKey, Changes>,
) {
    let mut rows = Vec::new();
    collect_changes(world, mark, &mut rows);
    for (key, changes) in rows {
        *keys.entry(key).or_default() |= changes;
    }
}

/// The rows of the five columns changed since `mark`, unsorted, a key once per column.
fn collect_changes(world: &World, mark: ChangeMark, rows: &mut Vec<(StableKey, Changes)>) {
    fn column<T: lactose_reflect::Component>(
        world: &World,
        mark: ChangeMark,
        changes: Changes,
        rows: &mut Vec<(StableKey, Changes)>,
    ) {
        if let Some(changed) = world.changed_since::<T>(mark) {
            rows.extend(changed.map(|(key, _)| (key, changes)));
        }
    }
    column::<Transform>(world, mark, Changes::TRANSFORM, rows);
    column::<Velocity>(world, mark, Changes::VELOCITY, rows);
    column::<RigidBody>(world, mark, Changes::BODY, rows);
    column::<Collider>(world, mark, Changes::COLLIDER, rows);
    column::<Joint>(world, mark, Changes::JOINT, rows);
}

impl Physics {
    /// The sync (`README.md`, "The tick", step 2).
    pub(crate) fn sync(&mut self, world: &mut World) {
        let Physics { backend, sync, cursor, mark, tick, commands, fresh, .. } = self;
        *tick = world.tick();
        let held = !sync.bodies.is_empty();
        // The log since the cursor, or nothing when a full diff is due.
        let incremental = match (*cursor, *mark) {
            (Some(mut cursor), Some(mark)) if world.is_current(mark) && !sync.diff_all => world
                .structural_log()
                .since(&mut cursor)
                .map(|entries| (entries.map(|entry| entry.change).collect::<Vec<_>>(), mark)),
            _ => None,
        };
        // Every body the solver held despawned, the pending entries a restore carried
        // included: the world was reloaded or rebuilt, in one frame if the rows are back.
        // The log alone shows it, since a row unloaded and loaded again is `Despawned` then
        // `Spawned` and the solver would otherwise keep its whole state (D124). A full diff
        // reads the same, from the log retained since the pass (`log_since_pass`).
        let reload = held && {
            let pending = sync.pending_log.iter().map(|entry| entry.change);
            match &incremental {
                Some((log, _)) => {
                    despawns_every_body(&sync.bodies, pending.chain(log.iter().copied()))
                }
                None => log_since_pass(world, *cursor).is_some_and(|entries| {
                    let logged = entries.map(|entry| entry.change);
                    despawns_every_body(&sync.bodies, pending.chain(logged))
                }),
            }
        };
        let mut pass = Pass {
            world: &mut *world,
            sync: &mut *sync,
            backend: &mut **backend,
            created: BTreeSet::new(),
            pushed_body: BTreeSet::new(),
            pushed_collider: BTreeSet::new(),
        };
        if reload {
            pass.empty(fresh);
        }
        pass.modes();
        match incremental {
            Some((log, mark)) if !reload => {
                let pending = mem::take(&mut pass.sync.pending_log);
                for change in pending.iter().map(|entry| entry.change).chain(log) {
                    pass.log_entry(change);
                }
                let mut rows = Vec::new();
                collect_changes(pass.world, mark, &mut rows);
                for (key, changes) in mem::take(&mut pass.sync.pending_keys) {
                    rows.push((key, changes));
                }
                // The ECS yields changed rows in storage order: sorted here, so the calls
                // are in key order under every layout.
                rows.sort_unstable_by_key(|&(key, _)| key);
                let mut rows = rows.into_iter().peekable();
                while let Some((key, mut changes)) = rows.next() {
                    while let Some((_, more)) = rows.next_if(|&(next, _)| next == key) {
                        changes |= more;
                    }
                    pass.changed(key, changes);
                }
            }
            _ => pass.full_diff(),
        }
        pass.retry_waiting_joints();
        pass.refresh_aliases();
        pass.take_commands(commands);
        // Removals that left the solver empty (a world unloaded a frame before it is
        // loaded): the solver is made fresh now, and the next sync creates whatever is
        // loaded by a full diff, in key order, as a fresh process's first sync does.
        let emptied = held && pass.sync.bodies.is_empty();
        if emptied && !reload {
            pass.empty(fresh);
        }
        sync.diff_all = emptied;
        *cursor = Some(world.structural_log().cursor());
        *mark = Some(world.mark());
    }

    /// The step (`README.md`, "The tick", step 3).
    pub(crate) fn step(
        &mut self,
        job: &mut Job<'_, '_>,
        dt: Scalar,
        contacts: &mut EventWriter<'_, ContactEvent>,
        triggers: &mut EventWriter<'_, TriggerEvent>,
    ) {
        let Physics { backend, config, tick, commands, output, .. } = self;
        output.clear();
        let input = StepInput { tick: *tick, dt, gravity: config.gravity, commands };
        let backend = &mut **backend;
        job.context().block_in_place(config.reserved_workers, || backend.step(&input, output));
        commands.clear();
        // The emission order is a function of the step's result alone: ascending by
        // (target, other, phase), whatever order the backend reported in. Ties, which only
        // a backend reporting a pair twice makes, are broken by value, so the one kept
        // does not depend on the backend's order either.
        output.contacts.sort_by(|a, b| {
            let pair = |event: &ContactEvent| (event.target, event.other, event.phase);
            let value = |event: &ContactEvent| {
                let (point, normal) =
                    (event.point.to_canonical_bytes(), event.normal.to_canonical_bytes());
                (point, normal, event.impulse.to_canonical_bytes())
            };
            pair(a).cmp(&pair(b)).then_with(|| value(a).cmp(&value(b)))
        });
        output.triggers.sort_by_key(|event| (event.target, event.other, event.phase));
        let (contact_count, trigger_count) = (output.contacts.len(), output.triggers.len());
        output.contacts.dedup_by_key(|event| (event.target, event.other, event.phase));
        output.triggers.dedup_by_key(|event| (event.target, event.other, event.phase));
        if output.contacts.len() != contact_count || output.triggers.len() != trigger_count {
            tracing::warn!(
                target: "physics",
                "the backend reported an event twice for one (target, other, phase); one kept"
            );
        }
        for &event in &output.contacts {
            contacts.emit(event);
        }
        for &event in &output.triggers {
            triggers.emit(event);
        }
    }

    /// The write-back (`README.md`, "The tick", step 4).
    pub(crate) fn write_back(&mut self, world: &mut World) {
        let tick = world.tick();
        let Physics { output, sync, .. } = self;
        // By key, and a key reported twice (a backend's broken promise) by value, so the
        // state kept does not depend on the backend's order.
        output.bodies.sort_by(|a, b| {
            let value = |state: &BodyState| {
                let (transform, velocity) = (state.transform, state.velocity);
                (
                    transform.translation.to_canonical_bytes(),
                    transform.rotation.to_canonical_bytes(),
                    velocity.linear.to_canonical_bytes(),
                    velocity.angular.to_canonical_bytes(),
                    state.teleported,
                )
            };
            a.key.cmp(&b.key).then_with(|| value(a).cmp(&value(b)))
        });
        sync.written.clear();
        for state in &output.bodies {
            let key = state.key;
            if sync.written.last().is_some_and(|&(last, ..)| last == key) {
                tracing::warn!(target: "physics", ?key, "the backend reported a body twice; one kept");
                continue;
            }
            // A row a command despawned at this sync point, or one that lost its
            // transform, is no body to write.
            let Some(mut transform) = world.get_mut::<Transform>(key) else { continue };
            transform.set_if_neq(state.transform);
            match world.get_mut::<Velocity>(key) {
                Some(mut velocity) => {
                    velocity.set_if_neq(state.velocity);
                }
                None => insert(world, key, state.velocity),
            }
            if state.teleported {
                match world.get_mut::<Teleported>(key) {
                    Some(mut teleported) => {
                        teleported.set_if_neq(Teleported(tick));
                    }
                    None => insert(world, key, Teleported(tick)),
                }
            }
            sync.written.push((key, state.transform, state.velocity));
        }
    }
}

/// The structural log since the sync's last pass, as far as the world retains it: the
/// entries after `cursor`; or, when the cursor is stale or trimmed past, every entry the
/// log retains, all of which came after the pass, since a new epoch (`set_tick`, a world
/// restore, a whole-world load, the rebase) starts with an empty log. An unload made
/// before the epoch began is not in it. `None` before the first pass.
pub(crate) fn log_since_pass(world: &World, cursor: Option<LogCursor>) -> Option<LogEntries<'_>> {
    let log = world.structural_log();
    let mut since = cursor?;
    log.since(&mut since).or_else(|| log.since(&mut log.cursor_at_start()))
}

/// Whether `changes` despawn every key of `bodies`.
fn despawns_every_body(
    bodies: &BTreeMap<StableKey, BodyHandle>,
    changes: impl Iterator<Item = StructuralChange>,
) -> bool {
    let despawned: BTreeSet<StableKey> = changes
        .filter_map(|change| match change {
            StructuralChange::Despawned { key, .. } if bodies.contains_key(&key) => Some(key),
            _ => None,
        })
        .collect();
    despawned.len() == bodies.len()
}

/// Writes a component the sync or the write-back maintains; the row exists, so a failure
/// is a broken invariant of the world's, logged rather than raised inside a system.
fn insert<T: lactose_reflect::Component>(world: &mut World, key: StableKey, value: T) {
    if let Err(error) = world.insert(key, value) {
        tracing::warn!(target: "physics", ?key, %error, "a physics row was not written");
    }
}

/// One sync: the world, the bookkeeping and the backend, and what this sync has already
/// told the backend, so that a key it created is not pushed again and a value it pushed
/// from the current row is not pushed twice.
struct Pass<'a> {
    world: &'a mut World,
    sync: &'a mut SyncState,
    backend: &'a mut dyn PhysicsBackend,
    created: BTreeSet<StableKey>,
    pushed_body: BTreeSet<StableKey>,
    pushed_collider: BTreeSet<StableKey>,
}

impl Pass<'_> {
    /// Region modes: every body of a region whose mode changed is frozen or thawed.
    fn modes(&mut self) {
        let modes: BTreeMap<RegionId, RegionMode> = self
            .world
            .regions()
            .filter_map(|region| Some((region, self.world.region_mode(region)?)))
            .collect();
        let changed: BTreeSet<RegionId> = modes
            .iter()
            .filter(|&(region, mode)| self.sync.modes.get(region) != Some(mode))
            .map(|(&region, _)| region)
            .collect();
        self.sync.modes = modes;
        if changed.is_empty() {
            return;
        }
        let keys: Vec<StableKey> = self
            .sync
            .bodies
            .keys()
            .copied()
            .filter(|&key| {
                self.world.region_of(key).is_some_and(|region| changed.contains(&region))
            })
            .collect();
        for key in keys {
            self.freeze(key);
        }
    }

    /// One structural log entry, read against the rows as they are now.
    fn log_entry(&mut self, change: StructuralChange) {
        match change {
            StructuralChange::Spawned { key, .. } => {
                self.body(key, None);
                self.joint(key);
                self.handles(key);
            }
            StructuralChange::Despawned { key, .. } => {
                self.body(key, None);
                self.joint(key);
            }
            StructuralChange::Added { key, component }
            | StructuralChange::Removed { key, component } => {
                if component == Transform::KEY
                    || component == RigidBody::KEY
                    || component == Collider::KEY
                {
                    self.body(key, Some(component));
                } else if component == ClaimedBy::KEY {
                    self.claim(key);
                } else if component == Joint::KEY {
                    self.joint(key);
                }
            }
            StructuralChange::Moved { key, .. } => {
                if self.sync.bodies.contains_key(&key) {
                    self.freeze(key);
                }
            }
        }
    }

    /// A key whose columns changed since the mark, or that a restore or a correction named.
    fn changed(&mut self, key: StableKey, changes: Changes) {
        if changes.contains(Changes::JOINT) {
            self.joint(key);
        }
        if self.created.contains(&key) {
            return;
        }
        if !self.sync.bodies.contains_key(&key) {
            // A row the backend refused is retried when it changes.
            if changes != Changes::JOINT && self.is_body_row(key) {
                self.create(key);
            }
            return;
        }
        if changes.contains(Changes::BODY) {
            self.push_body(key);
        }
        if changes.contains(Changes::COLLIDER) {
            self.push_collider(key);
        }
        let written = self.sync.written(key);
        if changes.contains(Changes::TRANSFORM)
            && let Some(&transform) = self.world.get::<Transform>(key)
            && written.is_none_or(|(last, _)| last != transform)
        {
            self.backend.set_transform(key, &transform);
        }
        if changes.contains(Changes::VELOCITY)
            && let Some(&velocity) = self.world.get::<Velocity>(key)
            && written.is_none_or(|(_, last)| last != velocity)
        {
            self.backend.set_velocity(key, &velocity);
        }
    }

    /// The handle rows of a key the bookkeeping holds, written again where its row lacks
    /// them: a row spawned while the solver kept its body or joint (a region unloaded and
    /// loaded in one frame, read from the log or by a full diff) carries none, and the
    /// columns mirror the bookkeeping.
    fn handles(&mut self, key: StableKey) {
        if let Some(&handle) = self.sync.bodies.get(&key)
            && self.world.get::<BodyHandle>(key) != Some(&handle)
        {
            insert(self.world, key, handle);
        }
        if let Some(&JointEntry { handle, .. }) = self.sync.joints.get(&key)
            && self.world.get::<JointHandle>(key) != Some(&handle)
        {
            insert(self.world, key, handle);
        }
    }

    /// Removes every body, which leaves every joint waiting, and restores the backend's
    /// bytes from `setup`: the solver is then a fresh one, and nothing the sync remembers
    /// of the old one survives (D124).
    fn empty(&mut self, fresh: &[u8]) {
        let keys: Vec<StableKey> = self.sync.bodies.keys().copied().collect();
        for key in keys {
            self.remove(key);
        }
        if let Err(error) = self.backend.restore(fresh) {
            // The backend's own bytes of its empty world: refusing them breaks its
            // obligations. The solver is empty all the same, only not fresh.
            tracing::warn!(target: "physics", %error, "the backend refused its empty world");
        }
        self.sync.written.clear();
    }

    /// Every row against the bookkeeping, in ascending key order, when the cursor or the
    /// mark is stale or the solver was emptied; it clears nothing itself, so sleep and warm
    /// starts survive wherever the rows agree, and writes back the handle rows of every
    /// body and joint it keeps.
    fn full_diff(&mut self) {
        self.sync.pending_log.clear();
        self.sync.pending_keys.clear();
        let mut keys: Vec<StableKey> = self.sync.bodies.keys().copied().collect();
        for column in self.world.columns::<RigidBody>() {
            keys.extend_from_slice(column.keys());
        }
        for column in self.world.columns::<Collider>() {
            keys.extend_from_slice(column.keys());
        }
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            match (self.sync.bodies.contains_key(&key), self.is_body_row(key)) {
                (false, true) => self.create(key),
                (true, false) => self.remove(key),
                (true, true) => {
                    self.freeze(key);
                    let body = self.effective(key);
                    self.backend.set_body(key, body.as_ref());
                    let collider = self.world.get::<Collider>(key).copied();
                    self.backend.set_collider(key, collider.as_ref());
                    if let Some(&transform) = self.world.get::<Transform>(key) {
                        self.backend.set_transform(key, &transform);
                    }
                    if let Some(&velocity) = self.world.get::<Velocity>(key) {
                        self.backend.set_velocity(key, &velocity);
                    }
                    self.handles(key);
                }
                (false, false) => {}
            }
        }
        let mut joints: Vec<StableKey> = self.sync.joints.keys().copied().collect();
        joints.extend(self.sync.waiting_joints.iter().copied());
        for column in self.world.columns::<Joint>() {
            joints.extend_from_slice(column.keys());
        }
        joints.sort_unstable();
        joints.dedup();
        for key in joints {
            self.joint(key);
            self.handles(key);
        }
    }

    /// Whether the row is a body: a `Transform` and a `RigidBody` or a `Collider`.
    fn is_body_row(&self, key: StableKey) -> bool {
        let shaped =
            self.world.get::<RigidBody>(key).is_some() || self.world.get::<Collider>(key).is_some();
        if shaped && self.world.get::<Transform>(key).is_none() {
            tracing::debug!(target: "physics", ?key, "a body row without a `Transform` is skipped");
            return false;
        }
        shaped
    }

    /// The row's `RigidBody` as the backend is to treat it: kinematic for a proxy, a
    /// prediction-world row without a claim.
    fn effective(&self, key: StableKey) -> Option<RigidBody> {
        let mut body = *self.world.get::<RigidBody>(key)?;
        if self.world.kind() == WorldKind::Prediction && self.world.get::<ClaimedBy>(key).is_none()
        {
            body.body_type = BodyType::Kinematic;
        }
        Some(body)
    }

    /// Whether the region `key` resides in is not `Simulated`, as the sync last read it.
    fn frozen_here(&self, key: StableKey) -> bool {
        let mode = self.world.region_of(key).and_then(|region| self.sync.modes.get(&region));
        mode != Some(&RegionMode::Simulated)
    }

    /// Reconciles whether the backend holds `key` frozen with its region's mode; a thawed
    /// body takes the row's velocity again.
    fn freeze(&mut self, key: StableKey) {
        let frozen = self.frozen_here(key);
        if frozen == self.sync.frozen.contains(&key) {
            return;
        }
        self.backend.set_frozen(key, frozen);
        if frozen {
            self.sync.frozen.insert(key);
        } else {
            self.sync.frozen.remove(&key);
            if let Some(&velocity) = self.world.get::<Velocity>(key) {
                self.backend.set_velocity(key, &velocity);
            }
        }
    }

    /// A presence change of the row: created, removed, or a component that is not the
    /// last of its kind pushed.
    fn body(&mut self, key: StableKey, component: Option<ComponentKey>) {
        match (self.sync.bodies.contains_key(&key), self.is_body_row(key)) {
            (false, true) => self.create(key),
            (true, false) => self.remove(key),
            (true, true) => {
                if component == Some(RigidBody::KEY) {
                    self.push_body(key);
                } else if component == Some(Collider::KEY) {
                    self.push_collider(key);
                }
            }
            (false, false) => {}
        }
    }

    /// `ClaimedBy` added to or removed from a row: a change of its effective body in a
    /// prediction world.
    fn claim(&mut self, key: StableKey) {
        if self.world.kind() == WorldKind::Prediction
            && self.sync.bodies.contains_key(&key)
            && self.world.get::<RigidBody>(key).is_some()
        {
            self.push_body(key);
        }
    }

    fn push_body(&mut self, key: StableKey) {
        if self.created.contains(&key) || !self.pushed_body.insert(key) {
            return;
        }
        let body = self.effective(key);
        self.backend.set_body(key, body.as_ref());
    }

    fn push_collider(&mut self, key: StableKey) {
        if self.created.contains(&key) || !self.pushed_collider.insert(key) {
            return;
        }
        let collider = self.world.get::<Collider>(key).copied();
        self.backend.set_collider(key, collider.as_ref());
    }

    fn create(&mut self, key: StableKey) {
        let Some(&transform) = self.world.get::<Transform>(key) else { return };
        let body = self.effective(key);
        let collider = self.world.get::<Collider>(key).copied();
        let velocity = self.world.get::<Velocity>(key).copied();
        let frozen = self.frozen_here(key);
        let desc = BodyDesc {
            body,
            collider: collider.as_ref(),
            transform: &transform,
            velocity: velocity.as_ref(),
            frozen,
        };
        match self.backend.create_body(key, &desc) {
            Ok(handle) => {
                self.sync.bodies.insert(key, handle);
                if frozen {
                    self.sync.frozen.insert(key);
                }
                if matches!(key, StableKey::Predicted(_)) {
                    // Looked up at the end of this sync, and at every one until it is bound.
                    self.sync.unbound.insert(key);
                }
                self.created.insert(key);
                insert(self.world, key, handle);
            }
            Err(error) => tracing::warn!(
                target: "physics",
                ?key,
                %error,
                "the backend refused a body; the row is retried when it changes"
            ),
        }
    }

    /// Removes the body of `key`; the backend removes its joints with it, and they wait.
    fn remove(&mut self, key: StableKey) {
        self.backend.remove_body(key);
        self.sync.bodies.remove(&key);
        self.sync.frozen.remove(&key);
        self.created.remove(&key);
        if let StableKey::Predicted(predicted) = key
            && !self.sync.unbound.remove(&key)
        {
            // Bound: its alias goes with it. The world keeps a bind until the rollback
            // horizon passes the despawn, so the lookup normally finds it.
            match self.world.alias(predicted) {
                Some(net) if self.sync.aliases.get(&net) == Some(&key) => {
                    self.sync.aliases.remove(&net);
                }
                _ => self.sync.aliases.retain(|_, &mut body| body != key),
            }
        }
        if self.world.get::<BodyHandle>(key).is_some()
            && let Err(error) = self.world.remove::<BodyHandle>(key)
        {
            tracing::warn!(target: "physics", ?key, %error, "a body handle was not removed");
        }
        let attached: Vec<StableKey> = self
            .sync
            .joints
            .iter()
            .filter(|(_, entry)| entry.body_a == key || entry.body_b == key)
            .map(|(&joint, _)| joint)
            .collect();
        for joint in attached {
            self.sync.joints.remove(&joint);
            self.remove_joint_handle(joint);
            self.sync.waiting_joints.insert(joint);
        }
    }

    /// A joint row that appeared, changed or went: removed and recreated on a change of
    /// value, created when it can be, waiting otherwise.
    fn joint(&mut self, key: StableKey) {
        let row = self.world.get::<Joint>(key).copied();
        match (self.sync.joints.get(&key), row) {
            (Some(entry), Some(joint)) if entry.joint == joint => {}
            (Some(_), Some(_)) => {
                self.remove_joint(key);
                self.try_joint(key);
            }
            (Some(_), None) => self.remove_joint(key),
            (None, Some(_)) => self.try_joint(key),
            (None, None) => {
                self.sync.waiting_joints.remove(&key);
            }
        }
    }

    fn remove_joint(&mut self, key: StableKey) {
        self.backend.remove_joint(key);
        self.sync.joints.remove(&key);
        self.remove_joint_handle(key);
    }

    fn remove_joint_handle(&mut self, key: StableKey) {
        if self.world.get::<JointHandle>(key).is_some()
            && let Err(error) = self.world.remove::<JointHandle>(key)
        {
            tracing::warn!(target: "physics", ?key, %error, "a joint handle was not removed");
        }
    }

    /// Creates the joint of `key` if both its bodies are in the solver; otherwise it waits,
    /// or, with an empty reference, is parked until its row changes.
    fn try_joint(&mut self, key: StableKey) {
        self.sync.waiting_joints.remove(&key);
        let Some(&joint) = self.world.get::<Joint>(key) else { return };
        let (Some(a), Some(b)) = (joint.body_a.get(), joint.body_b.get()) else {
            tracing::debug!(target: "physics", ?key, "a joint with an empty reference waits for its row to change");
            return;
        };
        let body_a = self.world.origin(a).unwrap_or(a);
        let body_b = self.world.origin(b).unwrap_or(b);
        if !(self.sync.bodies.contains_key(&body_a) && self.sync.bodies.contains_key(&body_b)) {
            self.sync.waiting_joints.insert(key);
            return;
        }
        match self.backend.create_joint(key, &joint, body_a, body_b) {
            Ok(handle) => {
                self.sync.joints.insert(key, JointEntry { handle, joint, body_a, body_b });
                insert(self.world, key, handle);
            }
            Err(error) => tracing::warn!(
                target: "physics",
                ?key,
                %error,
                "the backend refused a joint; the row is retried when it changes"
            ),
        }
    }

    fn retry_waiting_joints(&mut self) {
        let waiting: Vec<StableKey> = self.sync.waiting_joints.iter().copied().collect();
        for key in waiting {
            self.try_joint(key);
        }
    }

    /// The binds the world made since the last sync: each predicted body without a `Net`
    /// key is looked up, and leaves the unbound set once it has one. A bind is permanent, so
    /// a bound body is never looked up again.
    fn refresh_aliases(&mut self) {
        let SyncState { aliases, unbound, .. } = &mut *self.sync;
        unbound.retain(|&key| {
            let StableKey::Predicted(predicted) = key else { return false };
            match self.world.alias(predicted) {
                Some(net) => {
                    aliases.insert(net, key);
                    false
                }
                None => true,
            }
        });
    }

    /// This tick's command list: the commands a restore carried, then the channel's, in
    /// command order, each naming its body by origin key; a command naming no body is
    /// dropped.
    fn take_commands(&mut self, commands: &mut Vec<PhysicsCommand>) {
        commands.clear();
        let mut taken = mem::take(&mut self.sync.pending_commands);
        taken.extend(
            self.world.take_channel::<PhysicsCommands>().into_iter().map(|entry| entry.entry),
        );
        for mut command in taken {
            let key = command.body();
            let origin = match key {
                StableKey::Placeholder(_) => None,
                _ => self.world.origin(key),
            };
            match origin {
                Some(origin) if self.sync.bodies.contains_key(&origin) => {
                    *command.body_mut() = origin;
                    commands.push(command);
                }
                _ => tracing::debug!(
                    target: "physics",
                    ?key,
                    "a physics command dropped: its key names no body"
                ),
            }
        }
    }
}
