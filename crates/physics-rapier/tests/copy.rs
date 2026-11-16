//! The copy a prediction slot holds (D143): a backend restored from a copy steps as one
//! restored from its bytes does, and as the run it was taken from did, on a scene that
//! makes and removes joints of every kind, wakes sleeping bodies and replaces a shape while
//! older copies still share the old one; and a copy encodes the bytes the backend wrote. On
//! a second scene that churns its contact pairs, so that the copies carry retired pairs and
//! freed slots the bytes do not, a copy restores on another pool size, restores twice, and
//! restores into the backend that stepped on from it.

#![forbid(unsafe_code)]

use crate::support;

use lactose_core::ErrorKind;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{
    BackendCopy, Collider, Joint, JointKind, PhysicsBackend, PhysicsCommand, RigidBody, Shape,
    StepOutput, Transform, Velocity,
};
use support::*;

/// A sensor box the pile rolls through.
const SENSOR: u64 = 3;
/// The pile's bodies are `FIRST..FIRST + COUNT`.
const COUNT: u64 = 40;
/// Five boxes far from the pile, which come to rest and sleep: `SLEEPERS..SLEEPERS + 5`.
const SLEEPERS: u64 = 100;
/// A ball dropped onto the pile, then jointed to it.
const DROPPED: u64 = 400;
/// A box dropped onto a sleeper, waking it.
const WAKER: u64 = 401;
/// The tick a sleeper is woken by a velocity write.
const WAKE: u32 = 330;
/// The joints, one of each kind, then the one made later.
const FIXED: u64 = 1_000;
const SPHERICAL: u64 = 1_001;
const REVOLUTE: u64 = 1_002;
const PRISMATIC: u64 = 1_003;
const LATER: u64 = 1_004;

const TICKS: u32 = 420;
/// How many ticks each restore point is stepped on.
const SPAN: u32 = 12;

/// A scene: what is built before the first step, the calls and commands before each step,
/// a function of the tick alone so that every restored run replays them, and how many
/// ticks the live run steps.
struct Scene {
    build: fn(&mut dyn PhysicsBackend),
    script: fn(&mut dyn PhysicsBackend, u32) -> Vec<PhysicsCommand>,
    ticks: u32,
}

/// Joints of every kind, sleepers woken, a shape replaced.
const JOINTED: Scene = Scene { build, script, ticks: TICKS };

/// The scene before its first step.
fn build(b: &mut dyn PhysicsBackend) {
    create(b, key(FLOOR), None, Some(floor().with_events(Scalar::ZERO)), Transform::IDENTITY);
    let sensor = Collider::new(Shape::cuboid(v(1.5, 1.5, 1.5))).sensor();
    create(b, key(SENSOR), None, Some(sensor), at(3.0, 1.5, 0.0));
    create(b, key(PADDLE), Some(RigidBody::kinematic()), Some(ball()), at(-8.0, 0.5, 0.0));
    for i in 0..COUNT {
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
        let pose = Transform::new(Vec3::new(x, y, z), Quat::from_rotation_y(lean));
        create(b, key(FIRST + i), Some(RigidBody::dynamic()), Some(collider), pose);
    }
    for i in 0..5_u32 {
        let block = Collider::new(Shape::cuboid(v(0.4, 0.4, 0.4)));
        let pose = at(f64::from(i), 0.4, 20.0);
        create(b, key(SLEEPERS + u64::from(i)), Some(RigidBody::dynamic()), Some(block), pose);
    }
    let joints = [
        (FIXED, JointKind::Fixed, FIRST, FIRST + 1, None),
        (SPHERICAL, JointKind::Spherical, FIRST + 2, FIRST + 3, Some((-0.4, 0.4))),
        (REVOLUTE, JointKind::Revolute, FIRST + 4, FIRST + 14, Some((-0.6, 0.6))),
        (PRISMATIC, JointKind::Prismatic, FIRST + 16, FIRST + 17, Some((-0.3, 0.3))),
    ];
    for (n, kind, a, other, limits) in joints {
        let mut joint = Joint::new(kind, key(a), key(other))
            .with_anchors(v(0.55, 0.0, 0.0), v(-0.55, 0.0, 0.0))
            .with_axis(v(0.0, 0.0, 1.0));
        if let Some((low, high)) = limits {
            joint = joint.with_limits(s(low), s(high));
        }
        b.create_joint(key(n), &joint, key(a), key(other)).unwrap();
    }
}

/// The calls and commands before the step of `tick`.
fn script(b: &mut dyn PhysicsBackend, tick: u32) -> Vec<PhysicsCommand> {
    let mut commands = Vec::new();
    if tick < 260 {
        let x = -8.0 + 0.06 * f64::from(tick);
        b.set_transform(key(PADDLE), &at(x, 0.5, 0.3));
    }
    match tick {
        1 => {
            for i in (0..COUNT).step_by(3) {
                let impulse = v(0.3, 0.8, -0.2);
                commands.push(PhysicsCommand::ApplyImpulse { body: key(FIRST + i), impulse });
            }
        }
        60 => b.remove_body(key(FIRST + 5)),
        90 => b.remove_joint(key(SPHERICAL)),
        120 => {
            let collider = Collider::new(Shape::sphere(Scalar::HALF)).with_events(s(0.1));
            let body = Some(RigidBody::dynamic());
            create(b, key(DROPPED), body, Some(collider), at(0.0, 6.0, 0.0));
        }
        150 => {
            let joint = Joint::new(JointKind::Spherical, key(DROPPED), key(FIRST + 20))
                .with_anchors(v(0.0, -0.6, 0.0), v(0.0, 0.5, 0.0));
            b.create_joint(key(LATER), &joint, key(DROPPED), key(FIRST + 20)).unwrap();
        }
        180 => b.set_frozen(key(FIRST + 7), true),
        200 => {
            let torque = v(0.0, 2.0, 0.0);
            commands.push(PhysicsCommand::ApplyTorqueImpulse { body: key(FIRST + 3), torque });
        }
        210 => b.set_frozen(key(FIRST + 7), false),
        240 => b.set_body(key(FIRST + 8), Some(&RigidBody::kinematic())),
        // The copies taken before this tick share the shape it replaces.
        250 => {
            let replaced = Collider::new(Shape::cuboid(v(0.3, 0.5, 0.3)));
            b.set_collider(key(FIRST + 9), Some(&replaced));
        }
        270 => b.set_body(key(FIRST + 8), Some(&RigidBody::dynamic())),
        // Removes the revolute joint with its body.
        300 => b.remove_body(key(FIRST + 14)),
        320 => b.remove_joint(key(FIXED)),
        WAKE => {
            let velocity = Velocity { linear: v(2.0, 1.0, 0.0), angular: v(0.0, 3.0, 0.0) };
            b.set_velocity(key(SLEEPERS + 1), &velocity);
        }
        345 => b.remove_joint(key(PRISMATIC)),
        360 => {
            let block = Collider::new(Shape::cuboid(v(0.4, 0.4, 0.4)));
            create(b, key(WAKER), Some(RigidBody::dynamic()), Some(block), at(3.0, 1.5, 20.0));
        }
        380 => b.remove_body(key(SLEEPERS + 4)),
        _ => {}
    }
    commands
}

/// One scripted step of `tick` of `scene` and what it reported.
fn scripted(scene: &Scene, b: &mut dyn PhysicsBackend, tick: u32) -> Seen {
    let commands = (scene.script)(b, tick);
    Seen::from(&step(b, tick, &commands))
}

fn encoded(copy: &dyn BackendCopy) -> Vec<u8> {
    let mut bytes = Vec::new();
    copy.encode(&mut bytes);
    bytes
}

fn reported(seen: &Seen, n: u64) -> bool {
    seen.bodies.iter().any(|state| state.key == key(n))
}

/// What the live run held after every tick: index `t` is after tick `t`, index 0 the built
/// scene.
struct Run {
    bytes: Vec<Vec<u8>>,
    copies: Vec<Box<dyn BackendCopy>>,
    reports: Vec<Seen>,
}

/// The live run of `scene` on a pool of `threads`, with its bytes, copy and report after
/// every tick.
fn live(scene: &Scene, threads: usize) -> (Box<dyn PhysicsBackend>, Run) {
    let mut b = backend(threads);
    (scene.build)(&mut *b);
    let mut run = Run {
        bytes: vec![snapshot(&*b)],
        copies: vec![b.copy().unwrap()],
        reports: vec![Seen::from(&StepOutput::default())],
    };
    for tick in 1..=scene.ticks {
        run.reports.push(scripted(scene, &mut *b, tick));
        run.bytes.push(snapshot(&*b));
        run.copies.push(b.copy().unwrap());
    }
    for (t, (copy, bytes)) in run.copies.iter().zip(&run.bytes).enumerate() {
        assert!(encoded(&**copy) == *bytes, "the copy after tick {t} encodes the bytes");
    }
    (b, run)
}

/// Steps `b`, restored to the point after tick `t`, through the next [`SPAN`] ticks,
/// requiring its bytes and reports to be the live run's; `how` names the restore.
fn steps_as_the_run(scene: &Scene, b: &mut dyn PhysicsBackend, run: &Run, t: u32, how: &str) {
    assert!(snapshot(b) == run.bytes[t as usize], "{how} at {t} writes the bytes of its tick");
    for tick in t + 1..=t + SPAN {
        let seen = scripted(scene, b, tick);
        let at = tick as usize;
        assert!(seen == run.reports[at], "{how} at {t}: the report of tick {tick}");
        assert!(snapshot(b) == run.bytes[at], "{how} at {t}: the bytes after tick {tick}");
    }
}

// The only test that catches a restore keeping the live pipelines: it does so through the
// jointed scene's quiet tail after the paddle stops at tick 260 and its joint removals at
// ticks 300 (the revolute, with its body) and 320 (the fixed joint); the churned scene's
// cases pass with the live pipelines kept. An edit to `script` must keep that coverage.
#[test]
fn a_backend_restored_from_a_copy_steps_as_one_restored_from_its_bytes() {
    for threads in [1, 2, 4] {
        let (mut live, run) = live(&JOINTED, threads);
        // The scene does what it claims: a sleeper at rest is not reported until the write
        // wakes it, and the box dropped on another reports both.
        assert!(!reported(&run.reports[WAKE as usize - 1], SLEEPERS + 1), "asleep before");
        assert!(reported(&run.reports[WAKE as usize], SLEEPERS + 1), "woken by the write");
        assert!(!reported(&run.reports[359], SLEEPERS + 3), "asleep before the drop");
        assert!(
            (361..=380).any(|tick| reported(&run.reports[tick], SLEEPERS + 3)),
            "woken by the box dropped on it"
        );

        // Each restore point into one reused backend per form, so that a restore keeping
        // anything of what the backend stepped since shows.
        let mut from_copy = backend(threads);
        let mut from_bytes = backend(threads);
        for t in 0..=TICKS - SPAN {
            from_copy.restore_copy(&*run.copies[t as usize]).unwrap();
            from_bytes.restore(&run.bytes[t as usize]).unwrap();
            let how = format!("{threads} threads, a copy");
            steps_as_the_run(&JOINTED, &mut *from_copy, &run, t, &how);
            let how = format!("{threads} threads, bytes");
            steps_as_the_run(&JOINTED, &mut *from_bytes, &run, t, &how);
        }
        // Copies restored into the live backend newest first, every eighth tick, as a run of
        // rollbacks returns to them.
        for t in (0..=TICKS - SPAN).rev().step_by(8) {
            live.restore_copy(&*run.copies[t as usize]).unwrap();
            let how = format!("{threads} threads, the live backend from a copy");
            steps_as_the_run(&JOINTED, &mut *live, &run, t, &how);
        }
    }
}

#[test]
fn a_copy_from_another_backend_type_is_refused_and_changes_nothing() {
    struct Foreign;
    impl BackendCopy for Foreign {
        fn encode(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(b"not a world");
        }
    }
    let mut b = backend(1);
    build(&mut *b);
    let _ = scripted(&JOINTED, &mut *b, 1);
    let before = snapshot(&*b);
    let error = b.restore_copy(&Foreign).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(snapshot(&*b) == before, "the refused restore changed nothing");
}

/// A pile that churns its contact pairs (the first review's probe of D143's copy): balls
/// dropped every seventh tick and removed seven drops later, a tower of eighty boxes far
/// from the pile removed whole, rebuilt and removed again, joints made and removed, a
/// freeze, a type switch and a shape swap. Its copies carry the retired pairs and freed
/// slots its bytes leave out.
const CHURNED: Scene = Scene { build: churned::build, script: churned::script, ticks: 360 };

/// Restore points of the churned scene are every sixth tick.
const CHURNED_EVERY: usize = 6;

mod churned {
    use super::*;

    /// The pile's bodies are `FIRST..FIRST + COUNT`.
    const COUNT: u64 = 60;
    /// The tower's boxes are `TOWER..TOWER + TOWER_COUNT`.
    const TOWER: u64 = 3_000;
    const TOWER_COUNT: u32 = 80;
    /// The ball dropped at `tick` is `DROPS + tick`.
    const DROPS: u64 = 5_000;
    /// The joint made at `tick` is `LATE_JOINTS + tick`.
    const LATE_JOINTS: u64 = 9_000;
    const REVOLUTE: u64 = 1_002;

    pub(super) fn build(b: &mut dyn PhysicsBackend) {
        create(b, key(FLOOR), None, Some(floor()), Transform::IDENTITY);
        create(b, key(PADDLE), Some(RigidBody::kinematic()), Some(ball()), at(-8.0, 0.5, 0.0));
        for i in 0..COUNT {
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
            let pose = Transform::new(Vec3::new(x, y, z), Quat::from_rotation_y(lean));
            create(b, key(FIRST + i), Some(RigidBody::dynamic()), Some(collider), pose);
        }
        tower(b);
        let joint = Joint::new(JointKind::Revolute, key(FIRST + 4), key(FIRST + 14))
            .with_anchors(v(0.55, 0.0, 0.0), v(-0.55, 0.0, 0.0))
            .with_axis(v(0.0, 0.0, 1.0))
            .with_limits(s(-0.6), s(0.6));
        b.create_joint(key(REVOLUTE), &joint, key(FIRST + 4), key(FIRST + 14)).unwrap();
    }

    /// A leaning tower of boxes far from the pile, which collapses into a heap of pairs.
    fn tower(b: &mut dyn PhysicsBackend) {
        for i in 0..TOWER_COUNT {
            let (column, layer) = (i % 4, f64::from(i / 4));
            let x = 40.0 + 0.85 * f64::from(column % 2) + 0.03 * layer;
            let z = 0.85 * f64::from(column / 2);
            let y = 0.45 + 0.9 * layer;
            let block = Collider::new(Shape::cuboid(v(0.4, 0.4, 0.4)));
            let body = Some(RigidBody::dynamic());
            create(b, key(TOWER + u64::from(i)), body, Some(block), at(x, y, z));
        }
    }

    pub(super) fn script(b: &mut dyn PhysicsBackend, tick: u32) -> Vec<PhysicsCommand> {
        let mut commands = Vec::new();
        let x = -8.0 + 0.06 * f64::from(tick % 260);
        b.set_transform(key(PADDLE), &at(x, 0.5, 0.3));
        if tick.is_multiple_of(7) {
            let x = f64::from((tick * 37) % 9) - 4.0;
            let z = f64::from((tick * 53) % 9) - 4.0;
            let collider = Collider::new(Shape::sphere(s(0.3))).with_events(s(0.1));
            let body = Some(RigidBody::dynamic());
            create(b, key(DROPS + u64::from(tick)), body, Some(collider), at(x, 6.0, z));
        }
        if tick.is_multiple_of(7) && tick >= 56 {
            b.remove_body(key(DROPS + u64::from(tick - 49)));
        }
        if tick.is_multiple_of(13) {
            let body = key(FIRST + u64::from(tick % 40));
            commands.push(PhysicsCommand::ApplyImpulse { body, impulse: v(0.4, 1.5, -0.3) });
        }
        if tick % 50 == 25 {
            let (a, other) = (FIRST + u64::from(tick % 30), FIRST + 30 + u64::from(tick % 29));
            let joint = Joint::new(JointKind::Spherical, key(a), key(other))
                .with_anchors(v(0.0, 0.5, 0.0), v(0.0, -0.5, 0.0));
            let made = key(LATE_JOINTS + u64::from(tick));
            b.create_joint(made, &joint, key(a), key(other)).unwrap();
        }
        if tick.is_multiple_of(50) && tick >= 75 {
            b.remove_joint(key(LATE_JOINTS + u64::from(tick - 25)));
        }
        match tick % 60 {
            30 => b.set_frozen(key(FIRST + 7), true),
            0 => b.set_frozen(key(FIRST + 7), false),
            _ => {}
        }
        match tick % 80 {
            20 => b.set_body(key(FIRST + 8), Some(&RigidBody::kinematic())),
            60 => b.set_body(key(FIRST + 8), Some(&RigidBody::dynamic())),
            _ => {}
        }
        if tick % 90 == 45 {
            let shape = if (tick / 90).is_multiple_of(2) {
                Shape::cuboid(v(0.3, 0.5, 0.3))
            } else {
                Shape::sphere(s(0.4))
            };
            b.set_collider(key(FIRST + 9), Some(&Collider::new(shape)));
        }
        // The tower is removed whole, then rebuilt, so its pairs retire and come back.
        if tick == 150 || tick == 300 {
            for i in 0..TOWER_COUNT {
                b.remove_body(key(TOWER + u64::from(i)));
            }
        }
        if tick == 210 {
            tower(b);
        }
        commands
    }
}

#[test]
fn a_copy_restored_into_a_backend_of_another_pool_size_steps_as_the_run_it_was_taken_from() {
    for taken_on in [1, 2, 4] {
        let (_, run) = live(&CHURNED, taken_on);
        for restored_on in [1, 2, 4].into_iter().filter(|&threads| threads != taken_on) {
            let mut b = backend(restored_on);
            for t in (0..=CHURNED.ticks - SPAN).step_by(CHURNED_EVERY) {
                b.restore_copy(&*run.copies[t as usize]).unwrap();
                let how = format!("a copy taken on {taken_on} threads, restored on {restored_on}");
                steps_as_the_run(&CHURNED, &mut *b, &run, t, &how);
            }
        }
    }
}

#[test]
fn a_copy_restored_twice_steps_as_the_run_both_times_and_still_encodes_its_bytes() {
    for threads in [1, 2, 4] {
        let (_, run) = live(&CHURNED, threads);
        let mut b = backend(threads);
        for t in (0..=CHURNED.ticks - SPAN).step_by(CHURNED_EVERY * 3) {
            let copy = &*run.copies[t as usize];
            for time in ["first", "second"] {
                b.restore_copy(copy).unwrap();
                let how = format!("{threads} threads, a copy restored the {time} time");
                steps_as_the_run(&CHURNED, &mut *b, &run, t, &how);
            }
            assert!(encoded(copy) == run.bytes[t as usize], "the copy at {t} encodes its bytes");
        }
    }
}

#[test]
fn a_copy_restored_into_the_backend_that_stepped_on_from_it_steps_as_the_first_time() {
    for threads in [1, 2, 4] {
        let (_, run) = live(&CHURNED, threads);
        let mut b = backend(threads);
        (CHURNED.build)(&mut *b);
        let (mut t, mut restored) = (0, 0);
        while t + SPAN <= CHURNED.ticks {
            if t.is_multiple_of(30) {
                let copy = b.copy().unwrap();
                let how = format!("{threads} threads, stepped on from a copy");
                steps_as_the_run(&CHURNED, &mut *b, &run, t, &how);
                b.restore_copy(&*copy).unwrap();
                let how = format!("{threads} threads, the copy restored after stepping on");
                steps_as_the_run(&CHURNED, &mut *b, &run, t, &how);
                restored += 1;
                t += SPAN;
            } else {
                let seen = scripted(&CHURNED, &mut *b, t + 1);
                t += 1;
                assert!(seen == run.reports[t as usize], "{threads} threads: tick {t}");
            }
        }
        assert!(restored >= 10, "restored at {restored} points");
    }
}
