//! Events (`README.md`, "Events"): emitted ascending by (target, other, phase) whatever
//! order the backend reports them in; `Impact` above the threshold to a target whose own
//! collider has `events`, never to one without; `Started` and `Stopped` to both sides when
//! either has `events`; triggers likewise; a reader in `Stage::Post` sees each once; a
//! reader ordered before the step fails to build.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{Result, StableKey};
use lactose_ecs::{Events, SystemSet, WorldKind, system};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    Collider, ContactEvent, ContactPhase, PhysicsConfig, Shape, Transform, TriggerEvent, Velocity,
    register_systems, register_types, setup,
};
use lactose_sim::{Registration, SimulationBuilder, Stage, TypeTable};

fn at_x(x: f64) -> Transform {
    Transform::at(Vec3::new(Scalar::from_f64(x), Scalar::ZERO, Scalar::ZERO))
}

/// What the recorder saw, tick by tick, as (target, other, phase).
type Seen = Vec<Vec<(StableKey, StableKey, ContactPhase)>>;

fn contacts(threads: u32) -> (Seen, Seen) {
    let mut options = Options::new(WorldKind::Server);
    options.threads = threads;
    let mut h = options.build();
    let (a, b, c, sensor, d) = (key(1), key(2), key(3), key(4), key(5));
    let reporting = Collider::new(Shape::sphere(Scalar::HALF)).with_events(Scalar::HALF);
    let sliding = Velocity { linear: v(6, 0, 0), angular: Vec3::ZERO };
    h.spawn(a, (Transform::IDENTITY, Velocity::ZERO, floating(), reporting));
    // A tenth of a unit per tick: 0.75, 0.85, 0.95, then 1.05 from A, apart at the fourth.
    h.spawn(b, (at_x(0.65), sliding, floating(), sphere()));
    let above = Transform::at(Vec3::new(Scalar::ZERO, Scalar::from_f64(0.6), Scalar::ZERO));
    h.spawn(c, (above, Velocity::ZERO, floating(), sphere()));
    h.spawn(sensor, (at_x(10.0), Collider::new(Shape::sphere(Scalar::HALF)).sensor()));
    h.spawn(d, (at_x(10.5), Velocity::ZERO, floating(), sphere()));
    h = h.run(6);
    let recorded = h.recorded();
    let per_tick = |events: Vec<(u32, StableKey, StableKey, ContactPhase)>| -> Seen {
        (1..=6)
            .map(|tick| {
                events
                    .iter()
                    .filter(|event| event.0 == tick)
                    .map(|&(_, target, other, phase)| (target, other, phase))
                    .collect()
            })
            .collect()
    };
    let contacts =
        recorded.contacts.iter().map(|(t, e)| (*t, e.target, e.other, e.phase)).collect();
    let triggers =
        recorded.triggers.iter().map(|(t, e)| (*t, e.target, e.other, e.phase)).collect();
    (per_tick(contacts), per_tick(triggers))
}

#[test]
fn events_are_emitted_sorted_to_the_right_targets_and_seen_once() {
    use ContactPhase::{Impact, Started, Stopped};
    let (a, b, c, sensor, d) = (key(1), key(2), key(3), key(4), key(5));
    let (contacts_seen, triggers_seen) = contacts(1);
    // B slides away from A at six units per second: A has `events` and a threshold of one
    // half, so A gets an `Impact` in every step they touch and B, without `events`, never
    // does; both sides get `Started` and `Stopped`. B and C touch too, but neither has
    // `events`.
    assert_eq!(
        contacts_seen,
        [
            vec![
                (a, b, Started),
                (a, b, Impact),
                (a, c, Started),
                (b, a, Started),
                (c, a, Started)
            ],
            vec![(a, b, Impact)],
            vec![(a, b, Impact)],
            vec![(a, b, Stopped), (b, a, Stopped)],
            vec![],
            vec![],
        ]
    );
    assert_eq!(
        triggers_seen,
        [vec![(sensor, d, Started), (d, sensor, Started)], vec![], vec![], vec![], vec![], vec![]]
    );
    // The backend's report order depends on its pool; the emission order does not.
    assert_eq!(contacts(4), (contacts_seen, triggers_seen));
}

#[test]
fn an_impact_below_the_threshold_is_not_emitted() {
    let mut h = Harness::new(WorldKind::Server);
    let (a, b) = (key(1), key(2));
    let deaf = Collider::new(Shape::sphere(Scalar::HALF)).with_events(Scalar::from_i32(100));
    h.spawn(a, (Transform::IDENTITY, Velocity::ZERO, floating(), deaf));
    let sliding = Velocity { linear: v(6, 0, 0), angular: Vec3::ZERO };
    h.spawn(b, (at_x(0.6), sliding, floating(), sphere()));
    h = h.run(2);
    let phases: Vec<_> =
        h.recorded().contacts.iter().map(|(_, event)| (event.target, event.phase)).collect();
    assert_eq!(phases, [(a, ContactPhase::Started), (b, ContactPhase::Started)]);
}

/// A set in `Stage::Physics` with no edge after the step.
pub static EARLY: SystemSet = SystemSet::new("events_test::early");

#[system(name = "events_test::physics_reader", worlds = [server, prediction], in_set = EARLY)]
fn physics_reader(contacts: Events<ContactEvent>) {
    let _ = contacts.len();
}

#[system(name = "events_test::simulation_reader", worlds = [server, prediction])]
fn simulation_reader(triggers: Events<TriggerEvent>) {
    let _ = triggers.len();
}

fn build(register: fn(&mut Registration)) -> Result<()> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let backend = Box::new(Fake::new(CallLog::default()));
    SimulationBuilder::new(WorldKind::Server, types)
        .setup(move |world| setup(world, backend, PhysicsConfig::default()))
        .systems(register_systems)
        .systems(register)
        .build()
        .map(drop)
}

#[test]
fn a_reader_ordered_before_the_step_fails_to_build() {
    // In `Stage::Physics` without an edge after `sets::STEP`.
    let error = build(|r| {
        r.stage(Stage::Physics).sets(&[&EARLY]).add(&PHYSICS_READER);
    })
    .unwrap_err();
    assert!(error.to_string().contains("events_test::physics_reader"), "{error}");
    // In an earlier stage.
    let error = build(|r| {
        r.stage(Stage::Simulation).add(&SIMULATION_READER);
    })
    .unwrap_err();
    assert!(error.to_string().contains("events_test::simulation_reader"), "{error}");
    // In `Stage::Post`, with no edge at all, it builds.
    build(|r| {
        r.stage(Stage::Post).add(&SIMULATION_READER);
    })
    .unwrap();
}
