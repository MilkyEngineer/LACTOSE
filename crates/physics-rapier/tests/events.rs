//! Contact and trigger events from Rapier's: `Started` and `Stopped` to both sides when
//! either collider has `events`; `Impact` to a target whose own collider has `events`, above
//! its own threshold; the normal out of `other` toward `target`; triggers for sensors, which
//! push nothing; a removed body's contacts stop under its key; each (target, other, phase)
//! at most once per step, and the same events under every pool size.

#![forbid(unsafe_code)]

mod support;

use lactose_core::StableKey;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    ContactEvent, ContactPhase, PhysicsCommand, RigidBody, StepOutput, Transform, TriggerEvent,
    Velocity,
};
use support::*;

const GROUND: u64 = 1;
const LOUD: u64 = 2;
const DEAF: u64 = 3;
const QUIET: u64 = 4;

type Seen = Vec<(u32, StableKey, StableKey, ContactPhase)>;

fn record(seen: &mut Seen, tick: u32, output: &StepOutput) {
    let mut once = std::collections::BTreeSet::new();
    for event in &output.contacts {
        assert!(once.insert((event.target, event.other, event.phase)), "{event:?} twice");
        seen.push((tick, event.target, event.other, event.phase));
    }
}

/// Three balls dropped onto a floor with `events` and threshold zero: one with `events` and
/// threshold zero, one with `events` and a threshold no landing reaches, one without
/// `events`. After they rest, the first is teleported away and the second removed.
fn landing(threads: usize) -> (Seen, Vec<ContactEvent>) {
    let mut b = backend(threads);
    let ground = floor().with_events(Scalar::ZERO);
    create(&mut *b, key(GROUND), None, Some(ground), Transform::IDENTITY);
    let loud = ball().with_events(Scalar::ZERO);
    let deaf = ball().with_events(s(1e9));
    for (n, collider, x) in [(LOUD, loud, 0.0), (DEAF, deaf, 3.0), (QUIET, ball(), 6.0)] {
        create(&mut *b, key(n), Some(RigidBody::dynamic()), Some(collider), at(x, 1.0, 0.0));
    }
    let mut seen = Seen::new();
    let mut started = Vec::new();
    for tick in 1..=90 {
        let out = step(&mut *b, tick, &[]);
        record(&mut seen, tick, &out);
        started.extend(out.contacts.iter().filter(|e| e.phase == ContactPhase::Started));
    }
    let away = PhysicsCommand::Teleport { body: key(LOUD), transform: at(0.0, 20.0, 0.0) };
    b.remove_body(key(DEAF));
    let out = step(&mut *b, 91, &[away]);
    record(&mut seen, 91, &out);
    (seen, started)
}

#[test]
fn contacts_go_to_the_right_targets_with_the_right_normals() {
    use ContactPhase::{Impact, Started, Stopped};
    let (ground, loud, deaf, quiet) = (key(GROUND), key(LOUD), key(DEAF), key(QUIET));
    let (seen, started) = landing(1);
    let pairs = |phase: ContactPhase| -> Vec<(StableKey, StableKey)> {
        let mut pairs: Vec<_> =
            seen.iter().filter(|e| e.3 == phase).map(|&(_, t, o, _)| (t, o)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        pairs
    };
    // Every ball touches a floor with `events`: both sides of each pair start, once.
    let mut every = vec![
        (ground, loud),
        (ground, deaf),
        (ground, quiet),
        (loud, ground),
        (deaf, ground),
        (quiet, ground),
    ];
    every.sort_unstable();
    assert_eq!(pairs(Started), every);
    assert_eq!(seen.iter().filter(|e| e.3 == Started).count(), 6, "{seen:?}");
    // Impacts: to the floor from every ball, to the loud ball; never to the deaf one (its
    // threshold) or the quiet one (no `events`).
    let impacts = pairs(Impact);
    assert!(impacts.contains(&(loud, ground)) && impacts.contains(&(ground, quiet)));
    assert!(!impacts.iter().any(|&(target, _)| target == deaf || target == quiet));
    // The loud ball left by teleport and the deaf one was removed: both stopped, both sides,
    // the removed body under its key.
    let last: Vec<_> =
        seen.iter().filter(|e| e.0 == 91 && e.3 == Stopped).map(|e| (e.1, e.2)).collect();
    assert_eq!(last, [(ground, loud), (ground, deaf), (loud, ground), (deaf, ground)]);
    // The normal points out of `other`, toward `target`: up for a ball on the floor.
    for event in started {
        let up = event.normal.y;
        if event.target == ground {
            assert!(up < s(-0.99), "{event:?}");
        } else {
            assert!(up > s(0.99), "{event:?}");
        }
    }
}

#[test]
fn the_events_are_the_same_under_every_pool_size() {
    let reference = landing(1);
    for threads in [2, 4] {
        assert_eq!(landing(threads), reference, "{threads} threads");
    }
}

#[test]
fn a_sensor_reports_overlaps_to_both_sides_and_pushes_nothing() {
    use ContactPhase::{Started, Stopped};
    let mut b = backend(1);
    let (sensor, passer) = (key(1), key(2));
    create(&mut *b, sensor, None, Some(ball().sensor()), Transform::IDENTITY);
    let moving = Velocity { linear: v(6.0, 0.0, 0.0), angular: Vec3::ZERO };
    let desc = lactose_physics_api::BodyDesc {
        body: Some(floating()),
        collider: Some(&ball()),
        transform: &at(-1.5, 0.0, 0.0),
        velocity: Some(&moving),
        frozen: false,
    };
    b.create_body(passer, &desc).unwrap();
    let mut triggers: Vec<(u32, TriggerEvent)> = Vec::new();
    for tick in 1..=40 {
        let out = step(&mut *b, tick, &[]);
        assert!(out.contacts.is_empty());
        assert_eq!(state(&out, passer).unwrap().velocity, moving, "a sensor pushes nothing");
        triggers.extend(out.triggers.iter().map(|&event| (tick, event)));
    }
    let phases: Vec<_> = triggers.iter().map(|(_, e)| (e.target, e.other, e.phase)).collect();
    assert_eq!(
        phases,
        [
            (sensor, passer, Started),
            (passer, sensor, Started),
            (sensor, passer, Stopped),
            (passer, sensor, Stopped)
        ]
    );
    assert!(triggers[0].0 < triggers[2].0);
}

#[test]
fn removing_and_recreating_a_body_in_one_tick_keeps_one_body() {
    let mut b = backend(1);
    let body = key(1);
    create(&mut *b, body, Some(RigidBody::dynamic()), Some(ball()), at(0.0, 3.0, 0.0));
    step(&mut *b, 1, &[]);
    b.remove_body(body);
    create(&mut *b, body, Some(RigidBody::dynamic()), Some(ball()), at(5.0, 3.0, 0.0));
    let out = step(&mut *b, 2, &[]);
    assert_eq!(out.bodies.len(), 1);
    assert!(state(&out, body).unwrap().transform.translation.x == s(5.0));
    let mut hits = Vec::new();
    b.overlap(
        &lactose_physics_api::Shape::sphere(s(20.0)),
        &Transform::IDENTITY,
        &lactose_physics_api::QueryFilter::ALL,
        &mut hits,
    );
    assert_eq!(hits, [body]);
}
