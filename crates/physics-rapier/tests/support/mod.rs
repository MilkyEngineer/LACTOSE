//! What the tests that drive the backend directly share: a backend on a pool of a given
//! size, keys, values, and a step at sixty hertz under gravity of ten units downward.

#![allow(dead_code, reason = "each test file uses part of the shared module")]

use lactose_core::{NetId, StableKey, Tick};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    BodyDesc, BodyState, Collider, ContactEvent, Joint, JointKind, PhysicsBackend, PhysicsCommand,
    RigidBody, Shape, StepInput, StepOutput, Transform, TriggerEvent, Velocity,
};

pub fn backend(threads: usize) -> Box<dyn PhysicsBackend> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

/// A `Net` key.
pub fn key(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

pub fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

pub fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(s(x), s(y), s(z))
}

pub fn at(x: f64, y: f64, z: f64) -> Transform {
    Transform::at(v(x, y, z))
}

pub fn ball() -> Collider {
    Collider::new(Shape::sphere(Scalar::HALF))
}

/// A floor whose top face is at `y = 0`.
pub fn floor() -> Collider {
    Collider::new(Shape::cuboid(v(50.0, 1.0, 50.0))).with_offset(v(0.0, -1.0, 0.0))
}

/// A dynamic body without gravity: it moves only when pushed.
pub fn floating() -> RigidBody {
    let mut body = RigidBody::dynamic();
    body.gravity_scale = Scalar::ZERO;
    body
}

pub fn create(
    backend: &mut dyn PhysicsBackend,
    key: StableKey,
    body: Option<RigidBody>,
    collider: Option<Collider>,
    transform: Transform,
) {
    let desc = BodyDesc {
        body,
        collider: collider.as_ref(),
        transform: &transform,
        velocity: None,
        frozen: false,
    };
    backend.create_body(key, &desc).unwrap();
}

pub const GRAVITY: Vec3 = Vec3::new(Scalar::ZERO, Scalar::from_i32(-10), Scalar::ZERO);

/// One step of a sixtieth of a second.
pub fn step(
    backend: &mut dyn PhysicsBackend,
    tick: u32,
    commands: &[PhysicsCommand],
) -> StepOutput {
    let mut output = StepOutput::default();
    let input = StepInput { tick: Tick(tick), dt: s(1.0 / 60.0), gravity: GRAVITY, commands };
    backend.step(&input, &mut output);
    output
}

/// The reported state of `key`, if the step reported it.
pub fn state(output: &StepOutput, key: StableKey) -> Option<BodyState> {
    output.bodies.iter().find(|state| state.key == key).copied()
}

pub fn still(velocity: Velocity) -> bool {
    velocity == Velocity::ZERO
}

pub fn snapshot(backend: &dyn PhysicsBackend) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend.snapshot(&mut bytes).unwrap();
    bytes
}

// ---- A pile ----

/// The floor of [`Pile`].
pub const FLOOR: u64 = 1;
/// The kinematic paddle of [`Pile`], swept along x every tick.
pub const PADDLE: u64 = 5;
/// The first body of [`Pile`]; the bodies are `FIRST..FIRST + count`.
pub const FIRST: u64 = 10;
/// The pile's joints: fixed between the first two bodies, a limited spherical joint between
/// the next two.
pub const JOINTS: [u64; 2] = [1_000, 1_001];

/// One created thing of a [`Pile`].
#[derive(Clone, Copy)]
enum Part {
    Body(u64, Option<RigidBody>, Collider, Transform),
    Joint(u64, Joint),
}

/// A floor, `count` dynamic bodies of all three shapes in a grid of columns that topple,
/// a quarter of them with `events`, a kinematic paddle sweeping into them, and two joints:
/// the scene the determinism, snapshot and cost tests step.
pub struct Pile {
    parts: Vec<Part>,
}

impl Pile {
    pub fn new(count: u64) -> Self {
        let mut parts = vec![
            Part::Body(FLOOR, None, floor().with_events(Scalar::ZERO), Transform::IDENTITY),
            Part::Body(PADDLE, Some(RigidBody::kinematic()), ball(), at(-8.0, 0.5, 0.0)),
        ];
        for i in 0..count {
            let shape = match i % 3 {
                0 => Shape::sphere(s(0.45)),
                1 => Shape::cuboid(v(0.4, 0.4, 0.4)),
                _ => Shape::capsule(s(0.2), s(0.3)),
            };
            let mut collider = Collider::new(shape);
            if i % 4 == 0 {
                collider = collider.with_events(s(0.5));
            }
            let (column, layer) = (i % 25, i / 25);
            let lean = s(0.01) * Scalar::from_u64(layer);
            let x = s(1.1) * Scalar::from_u64(column % 5) - s(2.2) + lean;
            let z = s(1.1) * Scalar::from_u64(column / 5) - s(2.2);
            let y = s(0.6) + Scalar::from_u64(layer);
            let at = Transform::new(Vec3::new(x, y, z), lactose_math::Quat::from_rotation_y(lean));
            parts.push(Part::Body(FIRST + i, Some(RigidBody::dynamic()), collider, at));
        }
        if count >= 4 {
            let fixed = Joint::new(JointKind::Fixed, key(FIRST), key(FIRST + 1))
                .with_anchors(v(0.55, 0.0, 0.0), v(-0.55, 0.0, 0.0));
            let spherical = Joint::new(JointKind::Spherical, key(FIRST + 2), key(FIRST + 3))
                .with_anchors(v(0.55, 0.0, 0.0), v(-0.55, 0.0, 0.0))
                .with_limits(s(-0.4), s(0.4));
            parts.push(Part::Joint(JOINTS[0], fixed));
            parts.push(Part::Joint(JOINTS[1], spherical));
        }
        Self { parts }
    }

    /// Creates the pile's bodies, then its joints, each permuted by `seed` (zero keeps the
    /// listed order).
    pub fn create(&self, backend: &mut dyn PhysicsBackend, seed: u64) {
        let mut parts = self.parts.clone();
        if seed != 0 {
            // A fixed permutation from a test's own LCG.
            let mut state = seed;
            for i in (1..parts.len()).rev() {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                parts.swap(i, (state >> 33) as usize % (i + 1));
            }
        }
        for part in &parts {
            if let Part::Body(n, body, collider, transform) = *part {
                create(backend, key(n), body, Some(collider), transform);
            }
        }
        for part in &parts {
            if let Part::Joint(n, joint) = *part {
                let (a, b) = (joint.body_a.get().unwrap(), joint.body_b.get().unwrap());
                backend.create_joint(key(n), &joint, a, b).unwrap();
            }
        }
    }

    /// Creates the pile's body `n` alone, as [`create`](Self::create) does.
    pub fn create_body(&self, backend: &mut dyn PhysicsBackend, n: u64) {
        for part in &self.parts {
            if let Part::Body(m, body, collider, transform) = *part
                && m == n
            {
                create(backend, key(n), body, Some(collider), transform);
            }
        }
    }

    /// The writes and commands of tick `tick`: the paddle swept along x, an impulse, a
    /// teleport and a force at fixed ticks.
    pub fn drive(backend: &mut dyn PhysicsBackend, tick: u32) -> Vec<PhysicsCommand> {
        let x = s(-8.0) + s(0.05) * Scalar::from_u32(tick.min(200));
        backend.set_transform(key(PADDLE), &Transform::at(Vec3::new(x, s(0.5), Scalar::ZERO)));
        match tick {
            5 => {
                let impulse = v(0.0, 3.0, 1.0);
                vec![PhysicsCommand::ApplyImpulse { body: key(FIRST + 4), impulse }]
            }
            20 => {
                let transform = at(0.0, 8.0, 0.0);
                vec![PhysicsCommand::Teleport { body: key(FIRST + 5), transform }]
            }
            30 => {
                let force = v(40.0, 0.0, 0.0);
                vec![PhysicsCommand::ApplyForce { body: key(FIRST + 6), force }]
            }
            _ => Vec::new(),
        }
    }
}

/// A step's report in comparable form, bodies sorted by key.
#[derive(PartialEq, Debug)]
pub struct Seen {
    pub bodies: Vec<BodyState>,
    pub contacts: Vec<ContactEvent>,
    pub triggers: Vec<TriggerEvent>,
}

impl From<&StepOutput> for Seen {
    fn from(output: &StepOutput) -> Self {
        let mut bodies = output.bodies.clone();
        bodies.sort_by_key(|state| state.key);
        Self { bodies, contacts: output.contacts.clone(), triggers: output.triggers.clone() }
    }
}

/// Steps `backend` through ticks `from..=to` under [`Pile::drive`], returning each report.
pub fn run(backend: &mut dyn PhysicsBackend, from: u32, to: u32) -> Vec<Seen> {
    (from..=to)
        .map(|tick| {
            let commands = Pile::drive(backend, tick);
            Seen::from(&step(backend, tick, &commands))
        })
        .collect()
}
