//! Row writes and the write-back (`README.md`, "The tick"): a `Mut<Velocity>` write and an
//! impulse in one tick reach the backend as `set_velocity` then the command; a `Transform`
//! write sweeps a kinematic body and places a dynamic one; the step's own write-back is
//! never pushed back (probe F); a write in `Stage::Post` after the write-back is pushed
//! next tick; only rows whose bytes changed are stamped; `Teleported` holds the teleport's
//! tick across further ticks (probe T); a key despawned at the post sync point is skipped.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{Result, StableKey, Tick};
use lactose_ecs::WorldKind;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    BodyDesc, BodyHandle, Collider, Joint, JointHandle, PhysicsBackend, PhysicsCommand,
    QueryFilter, Ray, RayHit, RigidBody, Shape, ShapeHit, StepInput, StepOutput, Teleported,
    Transform, Velocity,
};
use lactose_sim::TickDelta;

fn moving(x: i32) -> Velocity {
    Velocity { linear: v(x, 0, 0), angular: v(0, 0, 0) }
}

#[test]
fn a_velocity_write_and_an_impulse_in_one_tick_reach_the_backend_in_that_order() {
    let mut h = Harness::new(WorldKind::Server);
    let body = key(1);
    h.spawn_body(body, at(0, 0, 0));
    h = h.run(1);
    h.take();
    let impulse = PhysicsCommand::ApplyImpulse { body, impulse: v(0, 2, 0) };
    h.script().at(2, When::Simulation, body, Action::WriteVelocity(moving(3))).at(
        2,
        When::Simulation,
        body,
        Action::Command(impulse),
    );
    h = h.run(1);
    assert_eq!(
        h.take(),
        [Call::SetVelocity(body, moving(3)), Call::Step { tick: Tick(2), commands: vec![impulse] }]
    );
    // The sync set the velocity, then the step added the impulse, then stepped.
    let expected = Velocity { linear: v(3, 2, 0), angular: v(0, 0, 0) };
    assert_eq!(*h.world().get::<Velocity>(body).unwrap(), expected);
}

#[test]
fn a_transform_write_sweeps_a_kinematic_body_and_places_a_dynamic_one() {
    let mut h = Harness::new(WorldKind::Server);
    let (swept, placed) = (key(1), key(2));
    h.spawn(swept, (at(0, 0, 0), Velocity::ZERO, RigidBody::kinematic(), sphere()));
    h.spawn_body(placed, at(10, 0, 0));
    h = h.run(1);
    h.take();
    h.script().at(2, When::Simulation, swept, Action::WriteTransform(at(1, 0, 0))).at(
        2,
        When::Simulation,
        placed,
        Action::WriteTransform(at(20, 0, 0)),
    );
    h = h.run(1);
    assert_eq!(
        sync_calls(&h.take()),
        [Call::SetTransform(swept, at(1, 0, 0)), Call::SetTransform(placed, at(20, 0, 0))]
    );
    // The kinematic body swept there, with the sweep's velocity, and is not teleported.
    let dt = h.world().resource::<TickDelta>().unwrap().0;
    assert_eq!(*h.world().get::<Transform>(swept).unwrap(), at(1, 0, 0));
    assert_eq!(h.world().get::<Velocity>(swept).unwrap().linear.x, Scalar::ONE / dt);
    assert!(h.world().get::<Teleported>(swept).is_none());
    // The dynamic body was placed without a sweep: the write-back wrote the tick.
    assert_eq!(*h.world().get::<Transform>(placed).unwrap(), at(20, 0, 0));
    assert_eq!(h.world().get::<Teleported>(placed), Some(&Teleported(Tick(2))));
}

#[test]
fn the_write_back_is_never_pushed_back() {
    const BODIES: u64 = 200;
    let mut h = Harness::new(WorldKind::Server);
    for n in 0..BODIES {
        h.spawn(key(n), (at(0, n as i32, 0), moving(1), floating(), sphere()));
    }
    h = h.run(2);
    h.take();
    for _ in 0..5 {
        let mark = h.world().mark();
        h = h.run(1);
        // Every body moved and was written back, so the stamps report every row...
        assert_eq!(h.world().changed_since::<Transform>(mark).unwrap().count(), BODIES as usize);
        // ...and the next sync pushes none of them: the step is all the backend was told.
        let calls = h.take();
        assert_eq!(calls.len(), 1, "{:?}", &calls[..calls.len().min(3)]);
        assert!(matches!(calls[0], Call::Step { .. }));
    }
    // A velocity equal to the last one written is not stamped at all.
    let mark = h.world().mark();
    h = h.run(1);
    assert_eq!(h.world().changed_since::<Velocity>(mark).unwrap().count(), 0);
}

#[test]
fn a_write_in_post_after_the_write_back_is_pushed_next_tick() {
    let mut h = Harness::new(WorldKind::Server);
    let (awake, asleep) = (key(1), key(2));
    h.spawn(awake, (at(0, 0, 0), moving(1), floating(), sphere()));
    // At rest: reported once, then asleep and not reported again.
    h.spawn_body(asleep, at(5, 0, 0));
    h = h.run(2);
    h.take();
    let pose = *h.world().get::<Transform>(asleep).unwrap();
    h.script()
        .at(3, When::Post, awake, Action::SetTransform(awake, at(-4, 0, 0)))
        // Back to its last pose: no entry in what the write-back wrote, so it is pushed.
        .at(3, When::Post, awake, Action::SetTransform(asleep, pose));
    h = h.run(1);
    // The post writes land after the write-back: the rows hold them, and this tick's sync
    // had nothing to push.
    assert_eq!(*h.world().get::<Transform>(awake).unwrap(), at(-4, 0, 0));
    assert_eq!(sync_calls(&h.take()), []);
    h = h.run(1);
    let calls = sync_calls(&h.take());
    assert_eq!(calls, [Call::SetTransform(awake, at(-4, 0, 0)), Call::SetTransform(asleep, pose)]);
    // The placed body moved on from there.
    let dt = h.world().resource::<TickDelta>().unwrap().0;
    assert_eq!(h.world().get::<Transform>(awake).unwrap().translation.x, Scalar::from_i32(-4) + dt);
}

#[test]
fn only_rows_whose_bytes_changed_are_stamped() {
    let mut h = Harness::new(WorldKind::Server);
    let (moves, rests) = (key(1), key(2));
    h.spawn(moves, (at(0, 0, 0), moving(1), floating(), sphere()));
    h.spawn_body(rests, at(5, 0, 0));
    let mark = h.world().mark();
    h = h.run(1);
    // Both bodies were reported by the first step; only the moved one's transform changed.
    let changed: Vec<_> =
        h.world().changed_since::<Transform>(mark).unwrap().map(|(k, _)| k).collect();
    assert_eq!(changed, [moves]);
    assert_eq!(h.world().changed_since::<Velocity>(mark).unwrap().count(), 0);
}

#[test]
fn a_body_without_velocity_gains_one_at_its_first_write_back() {
    let mut h = Harness::new(WorldKind::Server);
    let body = key(1);
    h.spawn(body, (at(0, 0, 0), RigidBody::dynamic(), sphere()));
    h = h.run(1);
    let velocity = h.world().get::<Velocity>(body).copied().unwrap();
    assert!(velocity.linear.y < Scalar::ZERO, "it fell under gravity");
    // The added velocity is the write-back's own: not pushed back.
    h.take();
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), []);
}

/// Probe T: a frame that runs several ticks before its passes loses no teleport.
#[test]
fn teleported_holds_the_tick_of_the_teleport_across_further_ticks() {
    let mut h = Harness::new(WorldKind::Prediction);
    let body = key(1);
    h.spawn_body(body, at(0, 0, 0));
    let teleport = |transform| PhysicsCommand::Teleport { body, transform };
    h.script().at(2, When::Simulation, body, Action::Command(teleport(at(0, 50, 0)))).at(
        5,
        When::Simulation,
        body,
        Action::Command(teleport(at(0, 60, 0))),
    );
    h = h.run(1);
    assert!(h.world().get::<Teleported>(body).is_none(), "never teleported");
    h = h.run(1);
    assert_eq!(h.world().get::<Teleported>(body), Some(&Teleported(Tick(2))));
    // Two more ticks in the same frame: the tick is still there for extract to compare.
    h = h.run(2);
    assert_eq!(h.world().get::<Teleported>(body), Some(&Teleported(Tick(2))));
    h = h.run(1);
    assert_eq!(h.world().get::<Teleported>(body), Some(&Teleported(Tick(5))));
    assert_eq!(*h.world().get::<Transform>(body).unwrap(), at(0, 60, 0));
}

#[test]
fn a_key_despawned_at_the_post_sync_point_is_skipped() {
    let mut h = Harness::new(WorldKind::Server);
    let (gone, kept) = (key(1), key(2));
    h.spawn(gone, (at(0, 0, 0), moving(1), floating(), sphere()));
    h.spawn(kept, (at(5, 0, 0), moving(1), floating(), sphere()));
    h = h.run(1);
    h.take();
    // Recorded in `Stage::Physics` after the step: the despawn applies at the sync point
    // that begins `Stage::Post`, before the write-back, which the step reported both to.
    h.script().at(2, When::AfterStep, kept, Action::Despawn(gone));
    let before = *h.world().get::<Transform>(kept).unwrap();
    h = h.run(1);
    assert!(!h.world().contains(gone));
    assert_ne!(*h.world().get::<Transform>(kept).unwrap(), before, "the other body was written");
    h = h.run(1);
    assert_eq!(sync_calls(&h.take()), [Call::RemoveBody(gone)]);
    assert!(h.world().get::<BodyHandle>(kept).is_some());
}

/// `README.md`, "The tick", step 3: in `Stage::Physics` a write to a body the step reports
/// is overwritten by the write-back, and one to a body it does not report is pushed at the
/// next sync (probe I4 of the implementation review).
#[test]
fn a_physics_stage_write_is_overwritten_if_the_step_reports_the_body_and_pushed_if_not() {
    let mut h = Harness::new(WorldKind::Server);
    let (awake, resting) = (key(1), key(2));
    h.spawn(awake, (at(0, 0, 0), moving(1), floating(), sphere()));
    // At rest: reported by the first step, asleep and unreported after it.
    h.spawn_body(resting, at(10, 0, 0));
    h = h.run(3);
    h.script().at(4, When::AfterStep, awake, Action::SetTransform(awake, at(0, 50, 0))).at(
        4,
        When::AfterStep,
        awake,
        Action::SetTransform(resting, at(10, 50, 0)),
    );
    h.take();
    h = h.run(1);
    let reported = *h.world().get::<Transform>(awake).unwrap();
    assert_ne!(reported, at(0, 50, 0), "the write-back overwrote it");
    assert_eq!(*h.world().get::<Transform>(resting).unwrap(), at(10, 50, 0), "it survived");
    h = h.run(1);
    let pushed: Vec<Call> = sync_calls(&h.take())
        .into_iter()
        .filter(|call| matches!(call, Call::SetTransform(..)))
        .collect();
    assert_eq!(pushed, [Call::SetTransform(resting, at(10, 50, 0))]);
}

/// A backend that reports the lowest key twice, the copy displaced upwards, first or last
/// in the list: a broken promise whose outcome must not depend on where it lands.
struct Twice {
    inner: Fake,
    first: bool,
}

impl PhysicsBackend for Twice {
    fn create_body(&mut self, key: StableKey, desc: &BodyDesc<'_>) -> Result<BodyHandle> {
        self.inner.create_body(key, desc)
    }
    fn remove_body(&mut self, key: StableKey) {
        self.inner.remove_body(key);
    }
    fn set_body(&mut self, key: StableKey, body: Option<&RigidBody>) {
        self.inner.set_body(key, body);
    }
    fn set_collider(&mut self, key: StableKey, collider: Option<&Collider>) {
        self.inner.set_collider(key, collider);
    }
    fn set_transform(&mut self, key: StableKey, transform: &Transform) {
        self.inner.set_transform(key, transform);
    }
    fn set_velocity(&mut self, key: StableKey, velocity: &Velocity) {
        self.inner.set_velocity(key, velocity);
    }
    fn set_frozen(&mut self, key: StableKey, frozen: bool) {
        self.inner.set_frozen(key, frozen);
    }
    fn create_joint(
        &mut self,
        key: StableKey,
        joint: &Joint,
        body_a: StableKey,
        body_b: StableKey,
    ) -> Result<JointHandle> {
        self.inner.create_joint(key, joint, body_a, body_b)
    }
    fn remove_joint(&mut self, key: StableKey) {
        self.inner.remove_joint(key);
    }
    fn step(&mut self, input: &StepInput<'_>, output: &mut StepOutput) {
        self.inner.step(input, output);
        let Some(&lowest) = output.bodies.iter().min_by_key(|state| state.key) else { return };
        let mut copy = lowest;
        copy.transform.translation.y += Scalar::from_i32(100);
        if self.first {
            output.bodies.insert(0, copy);
        } else {
            output.bodies.push(copy);
        }
    }
    fn snapshot(&self, out: &mut Vec<u8>) -> Result<()> {
        self.inner.snapshot(out)
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.restore(bytes)
    }
    fn cast_ray(&self, ray: &Ray, filter: &QueryFilter) -> Option<RayHit> {
        self.inner.cast_ray(ray, filter)
    }
    fn cast_shape(
        &self,
        shape: &Shape,
        pose: &Transform,
        direction: Vec3,
        max_distance: Scalar,
        filter: &QueryFilter,
    ) -> Option<ShapeHit> {
        self.inner.cast_shape(shape, pose, direction, max_distance, filter)
    }
    fn overlap(
        &self,
        shape: &Shape,
        pose: &Transform,
        filter: &QueryFilter,
        hits: &mut Vec<StableKey>,
    ) {
        self.inner.overlap(shape, pose, filter, hits);
    }
}

/// Probe I3 of the implementation review: a body reported twice is written the same way
/// whichever copy the backend lists first.
#[test]
fn a_body_reported_twice_is_written_the_same_whatever_the_order() {
    let rows_after = |first: bool| {
        let calls = CallLog::default();
        let backend = Box::new(Twice { inner: Fake::new(calls.clone()), first });
        let mut h = Options::new(WorldKind::Server).build_with(backend, calls);
        h.spawn(key(1), (at(0, 0, 0), moving(1), floating(), sphere()));
        h.spawn(key(2), (at(5, 0, 0), moving(1), floating(), sphere()));
        h = h.run(3);
        rows(h.world())
    };
    let (copy_first, copy_last) = (rows_after(true), rows_after(false));
    assert_eq!(copy_first, copy_last);
    // The copy with the smaller canonical bytes is kept: here the undisplaced one.
    assert_eq!(copy_first[0].1.translation.y, Scalar::ZERO);
}
