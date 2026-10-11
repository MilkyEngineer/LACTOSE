//! The test backend: a `PhysicsBackend` small enough to be obviously correct, and the
//! backend lane's model of what the trait asks.
//!
//! It records every call in order ([`CallLog`]), so a test asserts on exactly what the
//! sync and the step told it. It integrates dynamic bodies by explicit Euler (impulses,
//! forces and gravity, no collision response), sweeps kinematic bodies to the transform a
//! row write set, places a body without a sweep on a teleport or a row write, and reports
//! every awake dynamic body, every moved kinematic body and every placed body, so the
//! write-back has rows to write. Contacts are between bounding spheres: a pair that starts
//! or stops overlapping is `Started` or `Stopped` to both sides when either collider has
//! `events`, a touching pair whose relative speed exceeds a target's threshold is an
//! `Impact` to that target when its collider has `events`, and sensor pairs are triggers.
//! It reports each list in an order that depends on its "pool size", so the crate's sort is
//! what makes the emission order. Its snapshot is its whole state as bytes, written and
//! read field by field in canonical form, so a restore is a real decode.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use lactose_core::{Error, ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick};
use lactose_math::{Canonical, Quat, Scalar, Vec3};
use lactose_physics_api::{
    BodyDesc, BodyHandle, BodyState, BodyType, Collider, CollisionGroups, ContactEvent,
    ContactPhase, Joint, JointHandle, JointKind, PhysicsBackend, PhysicsCommand, QueryFilter, Ray,
    RayHit, RigidBody, Shape, ShapeHit, ShapeKind, StepInput, StepOutput, Transform, TriggerEvent,
    Velocity,
};

/// One call the backend received.
#[derive(Clone, PartialEq, Debug)]
pub enum Call {
    CreateBody {
        key: StableKey,
        body: Option<RigidBody>,
        collider: Option<Collider>,
        transform: Transform,
        velocity: Option<Velocity>,
        frozen: bool,
    },
    RemoveBody(StableKey),
    SetBody(StableKey, Option<RigidBody>),
    SetCollider(StableKey, Option<Collider>),
    SetTransform(StableKey, Transform),
    SetVelocity(StableKey, Velocity),
    SetFrozen(StableKey, bool),
    CreateJoint {
        key: StableKey,
        body_a: StableKey,
        body_b: StableKey,
    },
    RemoveJoint(StableKey),
    Step {
        tick: Tick,
        commands: Vec<PhysicsCommand>,
    },
    Snapshot,
    Restore,
}

impl Call {
    /// The key a body or joint call names; `None` for a step, snapshot or restore.
    pub fn key(&self) -> Option<StableKey> {
        match *self {
            Call::CreateBody { key, .. } | Call::CreateJoint { key, .. } => Some(key),
            Call::RemoveBody(key)
            | Call::SetBody(key, _)
            | Call::SetCollider(key, _)
            | Call::SetTransform(key, _)
            | Call::SetVelocity(key, _)
            | Call::SetFrozen(key, _)
            | Call::RemoveJoint(key) => Some(key),
            Call::Step { .. } | Call::Snapshot | Call::Restore => None,
        }
    }
}

/// The calls a [`Fake`] received, shared with the test that built it: the backend itself
/// is boxed inside `Physics`, which hands nothing of it out.
#[derive(Clone, Default)]
pub struct CallLog(Arc<Mutex<Vec<Call>>>);

impl CallLog {
    fn push(&self, call: Call) {
        self.0.lock().unwrap().push(call);
    }

    /// Every call so far.
    pub fn all(&self) -> Vec<Call> {
        self.0.lock().unwrap().clone()
    }

    /// Every call so far, and forgets them.
    pub fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    /// How many calls so far.
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// What the fake holds of one body.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Body {
    handle: u32,
    body: Option<RigidBody>,
    collider: Option<Collider>,
    transform: Transform,
    velocity: Velocity,
    frozen: bool,
    /// Where a kinematic body sweeps to in the next step.
    target: Option<Transform>,
    /// Placed without a sweep since the last step.
    placed: bool,
    awake: bool,
}

impl Body {
    /// What the solver does with it now: a frozen body or a collider alone is fixed.
    fn kind(&self) -> BodyType {
        match self.body {
            Some(body) if !self.frozen => body.body_type,
            _ => BodyType::Fixed,
        }
    }

    fn inverse_mass(&self) -> Scalar {
        let mass = self.body.map_or(Scalar::ZERO, |body| body.mass);
        if mass > Scalar::ZERO { Scalar::ONE / mass } else { Scalar::ONE }
    }
}

/// What the fake holds of one joint.
#[derive(Clone, Copy, PartialEq, Debug)]
struct JointRow {
    handle: u32,
    kind: JointKind,
    body_a: StableKey,
    body_b: StableKey,
}

/// The fake's whole state: what its snapshot bytes encode.
#[derive(Clone, Default, PartialEq, Debug)]
struct State {
    next_handle: u32,
    bodies: BTreeMap<StableKey, Body>,
    joints: BTreeMap<StableKey, JointRow>,
    /// Solid pairs overlapping after the last step, the lower key first.
    touching: BTreeSet<(StableKey, StableKey)>,
    /// Sensor pairs overlapping after the last step, likewise.
    sensing: BTreeSet<(StableKey, StableKey)>,
}

/// The test backend.
pub struct Fake {
    state: State,
    calls: CallLog,
    /// What a real backend's pool size would be: it changes the order the lists are
    /// reported in and nothing else.
    threads: u32,
    /// Whether the step looks for contacts at all (quadratic in the colliders; the cost
    /// scene turns it off).
    contacts: bool,
}

impl Fake {
    pub fn new(calls: CallLog) -> Self {
        Self { state: State::default(), calls, threads: 1, contacts: true }
    }

    /// The same fake with a "pool" of `threads` threads.
    pub fn with_threads(mut self, threads: u32) -> Self {
        self.threads = threads;
        self
    }

    /// The same fake without contact detection.
    pub fn without_contacts(mut self) -> Self {
        self.contacts = false;
        self
    }

    fn handle(&mut self) -> u32 {
        self.state.next_handle += 1;
        self.state.next_handle
    }

    /// The body of `key`, or `None` and a log line, as the trait asks of an unknown key.
    fn body_mut(&mut self, key: StableKey) -> Option<&mut Body> {
        let body = self.state.bodies.get_mut(&key);
        if body.is_none() {
            tracing::debug!(target: "physics", ?key, "the fake holds no such body");
        }
        body
    }

    /// Puts each list in a "pool"-dependent order: reversed for one thread, rotated by the
    /// thread count otherwise.
    fn scramble<T>(&self, list: &mut [T]) {
        if self.threads <= 1 {
            list.reverse();
        } else if !list.is_empty() {
            let by = self.threads as usize % list.len();
            list.rotate_left(by);
        }
    }
}

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

fn valid_transform(transform: &Transform) -> bool {
    let unit = (transform.rotation.length_squared() - Scalar::ONE).abs() < Scalar::from_f64(1e-6);
    transform.translation.is_finite() && transform.rotation.is_finite() && unit
}

fn valid_joint(joint: &Joint) -> bool {
    let limits = !joint.limited
        || (joint.limit_min.is_finite()
            && joint.limit_max.is_finite()
            && joint.limit_min <= joint.limit_max);
    let axis = match joint.kind {
        JointKind::Fixed | JointKind::Spherical => true,
        JointKind::Revolute | JointKind::Prismatic => joint.axis.try_normalize().is_some(),
    };
    joint.anchor_a.is_finite()
        && joint.anchor_b.is_finite()
        && joint.axis.is_finite()
        && axis
        && limits
}

fn valid_shape(shape: &Shape) -> bool {
    let positive = |value: Scalar| value > Scalar::ZERO && value.is_finite();
    match shape.kind {
        ShapeKind::Sphere => positive(shape.radius),
        ShapeKind::Capsule => positive(shape.radius) && shape.half_extents.y >= Scalar::ZERO,
        ShapeKind::Cuboid => shape.half_extents.to_array().into_iter().all(positive),
    }
}

/// The radius of a sphere around the shape's centre that holds it.
fn bounding_radius(shape: &Shape) -> Scalar {
    match shape.kind {
        ShapeKind::Sphere => shape.radius,
        ShapeKind::Capsule => shape.radius + shape.half_extents.y,
        ShapeKind::Cuboid => shape.half_extents.length(),
    }
}

/// Where a collider's bounding sphere is, and its radius.
fn bounds(body: &Body) -> Option<(Vec3, Scalar)> {
    let collider = body.collider?;
    let centre = body.transform.transform_point(collider.offset);
    Some((centre, bounding_radius(&collider.shape)))
}

/// Whether two colliders' groups let them interact.
fn interacts(a: CollisionGroups, b: CollisionGroups) -> bool {
    a.membership & b.filter != 0 && b.membership & a.filter != 0
}

/// Whether a query admits the collider of `key`.
fn admits(filter: &QueryFilter, key: StableKey, collider: &Collider) -> bool {
    filter.exclude != Some(key)
        && (filter.sensors || !collider.sensor)
        && interacts(filter.groups, collider.groups)
}

/// Where along the ray the sphere is first met, if within `max`.
fn ray_sphere(
    origin: Vec3,
    direction: Vec3,
    max: Scalar,
    centre: Vec3,
    radius: Scalar,
) -> Option<Scalar> {
    let offset = origin - centre;
    let b = offset.dot(direction);
    let c = offset.dot(offset) - radius * radius;
    if c <= Scalar::ZERO {
        return Some(Scalar::ZERO);
    }
    let discriminant = b * b - c;
    if discriminant < Scalar::ZERO {
        return None;
    }
    let t = -b - discriminant.sqrt();
    (t >= Scalar::ZERO && t <= max).then_some(t)
}

impl PhysicsBackend for Fake {
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle> {
        self.calls.push(Call::CreateBody {
            key,
            body: desc.body,
            collider: desc.collider.copied(),
            transform: *desc.transform,
            velocity: desc.velocity.copied(),
            frozen: desc.frozen,
        });
        assert!(!self.state.bodies.contains_key(&key), "create_body for a key the backend holds");
        assert!(desc.body.is_some() || desc.collider.is_some(), "a body with neither part");
        if !valid_transform(desc.transform) {
            return Err(invalid("not a valid transform"));
        }
        if desc.collider.is_some_and(|collider| !valid_shape(&collider.shape)) {
            return Err(invalid("not a valid shape"));
        }
        if desc.body.is_some_and(|body| body.mass.is_nan() || body.mass < Scalar::ZERO) {
            return Err(invalid("not a valid mass"));
        }
        let handle = self.handle();
        let body = Body {
            handle,
            body: desc.body,
            collider: desc.collider.copied(),
            transform: *desc.transform,
            velocity: desc.velocity.copied().unwrap_or(Velocity::ZERO),
            frozen: desc.frozen,
            target: None,
            placed: false,
            awake: true,
        };
        self.state.bodies.insert(key, body);
        Ok(BodyHandle::new(handle, 1))
    }

    fn remove_body(&mut self, key: StableKey) {
        self.calls.push(Call::RemoveBody(key));
        if self.state.bodies.remove(&key).is_none() {
            tracing::debug!(target: "physics", ?key, "the fake holds no such body");
            return;
        }
        self.state.joints.retain(|_, joint| joint.body_a != key && joint.body_b != key);
        self.state.touching.retain(|&(a, b)| a != key && b != key);
        self.state.sensing.retain(|&(a, b)| a != key && b != key);
    }

    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>) {
        self.calls.push(Call::SetBody(key, body.copied()));
        if let Some(held) = self.body_mut(key)
            && held.body.as_ref() != body
        {
            held.body = body.copied();
            held.awake = true;
        }
    }

    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>) {
        self.calls.push(Call::SetCollider(key, collider.copied()));
        if let Some(held) = self.body_mut(key)
            && held.collider.as_ref() != collider
        {
            held.collider = collider.copied();
            held.awake = true;
        }
    }

    fn set_transform(&mut self, key: StableKey, transform: &Transform) {
        self.calls.push(Call::SetTransform(key, *transform));
        if !valid_transform(transform) {
            return;
        }
        if let Some(held) = self.body_mut(key)
            && held.transform != *transform
        {
            if held.kind() == BodyType::Kinematic {
                held.target = Some(*transform);
            } else {
                held.transform = *transform;
                held.placed = true;
            }
            held.awake = true;
        }
    }

    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity) {
        self.calls.push(Call::SetVelocity(key, *velocity));
        if let Some(held) = self.body_mut(key)
            && held.velocity != *velocity
            && held.kind() != BodyType::Kinematic
        {
            held.velocity = *velocity;
            held.awake = true;
        }
    }

    fn set_frozen(&mut self, key: StableKey, frozen: bool) {
        self.calls.push(Call::SetFrozen(key, frozen));
        if let Some(held) = self.body_mut(key)
            && held.frozen != frozen
        {
            held.frozen = frozen;
            held.awake = true;
        }
    }

    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle> {
        self.calls.push(Call::CreateJoint { key, body_a, body_b });
        assert!(!self.state.joints.contains_key(&key), "create_joint for a key the backend holds");
        assert!(
            self.state.bodies.contains_key(&body_a) && self.state.bodies.contains_key(&body_b),
            "create_joint between bodies the backend does not hold"
        );
        if !valid_joint(joint) || body_a == body_b {
            return Err(invalid("not a valid joint"));
        }
        let handle = self.handle();
        self.state.joints.insert(key, JointRow { handle, kind: joint.kind, body_a, body_b });
        Ok(JointHandle::new(handle, 1))
    }

    fn remove_joint(&mut self, key: StableKey) {
        self.calls.push(Call::RemoveJoint(key));
        self.state.joints.remove(&key);
    }

    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        self.calls.push(Call::Step { tick: input.tick, commands: input.commands.to_vec() });
        assert!(
            output.bodies.is_empty() && output.contacts.is_empty() && output.triggers.is_empty()
        );
        let dt = input.dt;
        for command in input.commands {
            let Some(body) = self.body_mut(command.body()) else { continue };
            if let PhysicsCommand::Teleport { transform, .. } = *command {
                body.transform = transform;
                body.target = None;
                body.placed = true;
                body.awake = true;
                continue;
            }
            if body.kind() != BodyType::Dynamic {
                tracing::debug!(target: "physics", key = ?command.body(), "a push on a body that is not dynamic");
                continue;
            }
            let inverse_mass = body.inverse_mass();
            let velocity = &mut body.velocity;
            match *command {
                PhysicsCommand::ApplyImpulse { impulse, .. } => {
                    velocity.linear += impulse * inverse_mass;
                }
                PhysicsCommand::ApplyImpulseAt { impulse, point, .. } => {
                    velocity.linear += impulse * inverse_mass;
                    let arm = point - body.transform.translation;
                    velocity.angular += arm.cross(impulse);
                }
                PhysicsCommand::ApplyTorqueImpulse { torque, .. } => {
                    velocity.angular += torque;
                }
                PhysicsCommand::ApplyForce { force, .. } => {
                    velocity.linear += force * inverse_mass * dt;
                }
                PhysicsCommand::ApplyTorque { torque, .. } => {
                    velocity.angular += torque * dt;
                }
                PhysicsCommand::Teleport { .. } => unreachable!("handled above"),
            }
            body.awake = true;
        }
        for (&key, body) in &mut self.state.bodies {
            let mut moved = false;
            match body.kind() {
                BodyType::Dynamic if body.awake => {
                    let scale = body.body.map_or(Scalar::ONE, |rigid| rigid.gravity_scale);
                    let velocity = body.velocity.linear + input.gravity * scale * dt;
                    body.velocity.linear = velocity;
                    body.transform.translation += velocity * dt;
                    let at_rest = body.velocity == Velocity::ZERO;
                    body.awake = !(at_rest && body.body.is_some_and(|rigid| rigid.can_sleep));
                    moved = true;
                }
                BodyType::Kinematic => {
                    if let Some(target) = body.target.take() {
                        let travel = target.translation - body.transform.translation;
                        body.velocity = Velocity { linear: travel / dt, angular: Vec3::ZERO };
                        body.transform = target;
                        moved = true;
                    } else if body.velocity != Velocity::ZERO {
                        body.velocity = Velocity::ZERO;
                        moved = true;
                    }
                }
                _ => {}
            }
            if moved || body.placed {
                output.bodies.push(BodyState {
                    key,
                    transform: body.transform,
                    velocity: body.velocity,
                    teleported: body.placed,
                });
                body.placed = false;
            }
        }
        if self.contacts {
            self.find_contacts(output);
        }
        let mut bodies = std::mem::take(&mut output.bodies);
        let mut contacts = std::mem::take(&mut output.contacts);
        let mut triggers = std::mem::take(&mut output.triggers);
        self.scramble(&mut bodies);
        self.scramble(&mut contacts);
        self.scramble(&mut triggers);
        output.bodies = bodies;
        output.contacts = contacts;
        output.triggers = triggers;
    }

    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()> {
        self.calls.push(Call::Snapshot);
        encode(&self.state, out);
        Ok(())
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        self.calls.push(Call::Restore);
        let mut reader = Reader { bytes };
        let state = decode(&mut reader)?;
        if !reader.bytes.is_empty() {
            return Err(invalid("trailing bytes after the fake's state"));
        }
        self.state = state;
        Ok(())
    }

    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        for (&key, body) in &self.state.bodies {
            let (Some(collider), Some((centre, radius))) = (body.collider, bounds(body)) else {
                continue;
            };
            if !admits(filter, key, &collider) {
                continue;
            }
            let Some(distance) =
                ray_sphere(ray.origin, ray.direction, ray.max_distance, centre, radius)
            else {
                continue;
            };
            // Ascending keys and a strict comparison: a tie goes to the lowest key.
            if best.is_none_or(|hit| distance < hit.distance) {
                let point = ray.origin + ray.direction * distance;
                let normal = (point - centre).try_normalize().unwrap_or(-ray.direction);
                best = Some(RayHit { body: key, distance, point, normal });
            }
        }
        best
    }

    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        let inflate = bounding_radius(shape);
        let mut best: Option<ShapeHit> = None;
        for (&key, body) in &self.state.bodies {
            let (Some(collider), Some((centre, radius))) = (body.collider, bounds(body)) else {
                continue;
            };
            if !admits(filter, key, &collider) {
                continue;
            }
            let Some(distance) =
                ray_sphere(pose.translation, direction, max_distance, centre, radius + inflate)
            else {
                continue;
            };
            if best.is_none_or(|hit| distance < hit.distance) {
                let reached = pose.translation + direction * distance;
                let normal = (reached - centre).try_normalize().unwrap_or(-direction);
                best =
                    Some(ShapeHit { body: key, distance, point: centre + normal * radius, normal });
            }
        }
        best
    }

    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        let reach = bounding_radius(shape);
        let mut found = Vec::new();
        for (&key, body) in &self.state.bodies {
            let (Some(collider), Some((centre, radius))) = (body.collider, bounds(body)) else {
                continue;
            };
            if admits(filter, key, &collider) && pose.translation.distance(centre) < radius + reach
            {
                found.push(key);
            }
        }
        // With a "pool", out of order, which `Physics::overlap` puts right; alone, as the
        // trait asks.
        if self.threads > 1 {
            self.scramble(&mut found);
        }
        hits.extend(found);
    }
}

impl Fake {
    /// Pairs of bounding spheres that overlap now against those that overlapped after the
    /// last step.
    fn find_contacts(&mut self, output: &mut StepOutput) {
        let colliders: Vec<(StableKey, Body, Collider, Vec3, Scalar)> = self
            .state
            .bodies
            .iter()
            .filter_map(|(&key, body)| {
                let (centre, radius) = bounds(body)?;
                Some((key, *body, body.collider?, centre, radius))
            })
            .collect();
        let mut touching = BTreeSet::new();
        let mut sensing = BTreeSet::new();
        for (i, a) in colliders.iter().enumerate() {
            for b in &colliders[i + 1..] {
                let both_fixed = a.1.kind() == BodyType::Fixed && b.1.kind() == BodyType::Fixed;
                if both_fixed
                    || !interacts(a.2.groups, b.2.groups)
                    || a.3.distance(b.3) >= a.4 + b.4
                {
                    continue;
                }
                if a.2.sensor || b.2.sensor {
                    sensing.insert((a.0, b.0));
                } else {
                    touching.insert((a.0, b.0));
                }
            }
        }
        let find = |key: StableKey| colliders.iter().find(|entry| entry.0 == key).copied();
        for (phase, pairs) in [
            (
                ContactPhase::Started,
                touching.difference(&self.state.touching).copied().collect::<Vec<_>>(),
            ),
            (ContactPhase::Stopped, self.state.touching.difference(&touching).copied().collect()),
        ] {
            for (a, b) in pairs {
                let events_a = self
                    .state
                    .bodies
                    .get(&a)
                    .and_then(|body| body.collider)
                    .is_some_and(|c| c.events);
                let events_b = self
                    .state
                    .bodies
                    .get(&b)
                    .and_then(|body| body.collider)
                    .is_some_and(|c| c.events);
                if !(events_a || events_b) {
                    continue;
                }
                for (target, other) in [(a, b), (b, a)] {
                    let (point, normal) = match (phase, find(target), find(other)) {
                        (ContactPhase::Started, Some(t), Some(o)) => {
                            let normal = (t.3 - o.3).try_normalize().unwrap_or(Vec3::Y);
                            ((t.3 + o.3) * Scalar::HALF, normal)
                        }
                        _ => (Vec3::ZERO, Vec3::ZERO),
                    };
                    output.contacts.push(ContactEvent {
                        target,
                        other,
                        phase,
                        point,
                        normal,
                        impulse: Scalar::ZERO,
                    });
                }
            }
        }
        for &(a, b) in &touching {
            let (Some(ea), Some(eb)) = (find(a), find(b)) else { continue };
            let impulse = (ea.1.velocity.linear - eb.1.velocity.linear).length();
            for (target, other) in [(ea, eb), (eb, ea)] {
                if target.2.events && impulse > target.2.impact_threshold {
                    let normal = (target.3 - other.3).try_normalize().unwrap_or(Vec3::Y);
                    let point = (target.3 + other.3) * Scalar::HALF;
                    output.contacts.push(ContactEvent {
                        target: target.0,
                        other: other.0,
                        phase: ContactPhase::Impact,
                        point,
                        normal,
                        impulse,
                    });
                }
            }
        }
        for (phase, pairs) in [
            (
                ContactPhase::Started,
                sensing.difference(&self.state.sensing).copied().collect::<Vec<_>>(),
            ),
            (ContactPhase::Stopped, self.state.sensing.difference(&sensing).copied().collect()),
        ] {
            for (a, b) in pairs {
                output.triggers.push(TriggerEvent { target: a, other: b, phase });
                output.triggers.push(TriggerEvent { target: b, other: a, phase });
            }
        }
        self.state.touching = touching;
        self.state.sensing = sensing;
    }
}

// ---- The snapshot bytes: the state, field by field, canonical little-endian ----

fn encode(state: &State, out: &mut Vec<u8>) {
    let mut w = Writer { out };
    w.u32(state.next_handle);
    w.u32(state.bodies.len() as u32);
    for (&key, body) in &state.bodies {
        w.key(key);
        w.u32(body.handle);
        w.flag(body.body.is_some());
        if let Some(rigid) = &body.body {
            w.rigid_body(rigid);
        }
        w.flag(body.collider.is_some());
        if let Some(collider) = &body.collider {
            w.collider(collider);
        }
        w.transform(&body.transform);
        w.velocity(&body.velocity);
        w.flag(body.frozen);
        w.flag(body.target.is_some());
        if let Some(target) = &body.target {
            w.transform(target);
        }
        w.flag(body.placed);
        w.flag(body.awake);
    }
    w.u32(state.joints.len() as u32);
    for (&key, joint) in &state.joints {
        w.key(key);
        w.u32(joint.handle);
        w.u8(joint.kind as u8);
        w.key(joint.body_a);
        w.key(joint.body_b);
    }
    for pairs in [&state.touching, &state.sensing] {
        w.u32(pairs.len() as u32);
        for &(a, b) in pairs {
            w.key(a);
            w.key(b);
        }
    }
}

fn decode(r: &mut Reader<'_>) -> Result<State> {
    let mut state = State { next_handle: r.u32()?, ..State::default() };
    for _ in 0..r.u32()? {
        let key = r.key()?;
        let handle = r.u32()?;
        let body = if r.flag()? { Some(r.rigid_body()?) } else { None };
        let collider = if r.flag()? { Some(r.collider()?) } else { None };
        let transform = r.transform()?;
        let velocity = r.velocity()?;
        let frozen = r.flag()?;
        let target = if r.flag()? { Some(r.transform()?) } else { None };
        let placed = r.flag()?;
        let awake = r.flag()?;
        let body =
            Body { handle, body, collider, transform, velocity, frozen, target, placed, awake };
        state.bodies.insert(key, body);
    }
    for _ in 0..r.u32()? {
        let key = r.key()?;
        let handle = r.u32()?;
        let kind = match r.u8()? {
            0 => JointKind::Fixed,
            1 => JointKind::Spherical,
            2 => JointKind::Revolute,
            3 => JointKind::Prismatic,
            _ => return Err(invalid("not a joint kind")),
        };
        let joint = JointRow { handle, kind, body_a: r.key()?, body_b: r.key()? };
        state.joints.insert(key, joint);
    }
    for pairs in [&mut state.touching, &mut state.sensing] {
        for _ in 0..r.u32()? {
            pairs.insert((r.key()?, r.key()?));
        }
    }
    Ok(state)
}

struct Writer<'a> {
    out: &'a mut Vec<u8>,
}

impl Writer<'_> {
    fn u8(&mut self, value: u8) {
        self.out.push(value);
    }

    fn flag(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    fn scalar(&mut self, value: Scalar) {
        self.out.extend_from_slice(&value.to_canonical_bytes());
    }

    fn vec3(&mut self, value: Vec3) {
        self.out.extend_from_slice(value.to_canonical_bytes().as_ref());
    }

    fn quat(&mut self, value: Quat) {
        self.out.extend_from_slice(value.to_canonical_bytes().as_ref());
    }

    fn key(&mut self, key: StableKey) {
        match key {
            StableKey::Net(net) => {
                self.u8(1);
                self.out.extend_from_slice(&net.to_bits().to_le_bytes());
            }
            StableKey::Predicted(predicted) => {
                self.u8(2);
                self.u32(predicted.player.0);
                self.u32(predicted.input_tick.0);
                self.u32(predicted.ordinal);
            }
            StableKey::Placeholder(_) => unreachable!("a placeholder never reaches the backend"),
        }
    }

    fn transform(&mut self, value: &Transform) {
        self.vec3(value.translation);
        self.quat(value.rotation);
    }

    fn velocity(&mut self, value: &Velocity) {
        self.vec3(value.linear);
        self.vec3(value.angular);
    }

    fn rigid_body(&mut self, value: &RigidBody) {
        for scalar in [value.mass, value.linear_damping, value.angular_damping, value.gravity_scale]
        {
            self.scalar(scalar);
        }
        self.u8(value.body_type as u8);
        self.u8(value.locked_axes);
        self.flag(value.can_sleep);
    }

    fn collider(&mut self, value: &Collider) {
        self.vec3(value.shape.half_extents);
        self.scalar(value.shape.radius);
        self.u8(value.shape.kind as u8);
        self.vec3(value.offset);
        self.quat(value.rotation);
        for scalar in [value.friction, value.restitution, value.density, value.impact_threshold] {
            self.scalar(scalar);
        }
        self.u32(value.groups.membership);
        self.u32(value.groups.filter);
        self.flag(value.sensor);
        self.flag(value.events);
        self.out.extend_from_slice(&value.material.to_le_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        if self.bytes.len() < N {
            return Err(invalid("the fake's snapshot ends early"));
        }
        let (head, rest) = self.bytes.split_at(N);
        self.bytes = rest;
        Ok(head.try_into().expect("N bytes"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn flag(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("not a flag")),
        }
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn scalar(&mut self) -> Result<Scalar> {
        Ok(Scalar::from_canonical_bytes(self.take()?))
    }

    fn vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.scalar()?, self.scalar()?, self.scalar()?))
    }

    fn quat(&mut self) -> Result<Quat> {
        Ok(Quat::new(self.scalar()?, self.scalar()?, self.scalar()?, self.scalar()?))
    }

    fn key(&mut self) -> Result<StableKey> {
        match self.u8()? {
            1 => {
                let bits = u64::from_le_bytes(self.take()?);
                NetId::from_bits(bits).map(StableKey::Net).ok_or_else(|| invalid("not a NetId"))
            }
            2 => Ok(StableKey::Predicted(PredictionKey {
                player: PlayerId(self.u32()?),
                input_tick: Tick(self.u32()?),
                ordinal: self.u32()?,
            })),
            _ => Err(invalid("not a key")),
        }
    }

    fn transform(&mut self) -> Result<Transform> {
        Ok(Transform::new(self.vec3()?, self.quat()?))
    }

    fn velocity(&mut self) -> Result<Velocity> {
        Ok(Velocity { linear: self.vec3()?, angular: self.vec3()? })
    }

    fn body_type(&mut self) -> Result<BodyType> {
        match self.u8()? {
            0 => Ok(BodyType::Dynamic),
            1 => Ok(BodyType::Kinematic),
            2 => Ok(BodyType::Fixed),
            _ => Err(invalid("not a body type")),
        }
    }

    fn rigid_body(&mut self) -> Result<RigidBody> {
        let mut body = RigidBody::dynamic();
        body.mass = self.scalar()?;
        body.linear_damping = self.scalar()?;
        body.angular_damping = self.scalar()?;
        body.gravity_scale = self.scalar()?;
        body.body_type = self.body_type()?;
        body.locked_axes = self.u8()?;
        body.can_sleep = self.flag()?;
        Ok(body)
    }

    fn collider(&mut self) -> Result<Collider> {
        let half_extents = self.vec3()?;
        let radius = self.scalar()?;
        let mut shape = match self.u8()? {
            0 => Shape::sphere(radius),
            1 => Shape::capsule(half_extents.y, radius),
            2 => Shape::cuboid(half_extents),
            _ => return Err(invalid("not a shape kind")),
        };
        shape.half_extents = half_extents;
        shape.radius = radius;
        let mut collider = Collider::new(shape);
        collider.offset = self.vec3()?;
        collider.rotation = self.quat()?;
        collider.friction = self.scalar()?;
        collider.restitution = self.scalar()?;
        collider.density = self.scalar()?;
        collider.impact_threshold = self.scalar()?;
        collider.groups = CollisionGroups::new(self.u32()?, self.u32()?);
        collider.sensor = self.flag()?;
        collider.events = self.flag()?;
        collider.material = u16::from_le_bytes(self.take()?);
        Ok(collider)
    }
}
