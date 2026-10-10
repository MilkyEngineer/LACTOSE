//! The step and the snapshot bytes are functions of the rows, not of the pool or of what
//! ran before: a pile stepped on one, two and four solver threads, and built in different
//! orders with writes, removals and re-creations that end at the same rows, reports the
//! same bodies and events every tick and writes the same bytes; and what it reports does
//! not change when an unrelated scene is awake far away.

#![forbid(unsafe_code)]

mod support;

use lactose_physics_api::{Joint, JointKind, PhysicsBackend, RigidBody, Velocity};
use support::*;

const BODIES: u64 = 150;
const TICKS: u32 = 120;

/// The pile built with `seed`'s order on `threads` threads, stepped `TICKS` ticks: each
/// report and the bytes after ticks 1, 60 and the last.
fn pile(threads: usize, seed: u64, noise: bool) -> (Vec<Seen>, Vec<Vec<u8>>) {
    let mut b = backend(threads);
    if noise {
        noisy_prelude(&mut *b);
    }
    Pile::new(BODIES).create(&mut *b, seed);
    if noise {
        noisy_writes(&mut *b);
    }
    let mut seen = run(&mut *b, 1, 1);
    let mut bytes = vec![snapshot(&*b)];
    seen.extend(run(&mut *b, 2, 60));
    bytes.push(snapshot(&*b));
    seen.extend(run(&mut *b, 61, TICKS));
    bytes.push(snapshot(&*b));
    (seen, bytes)
}

/// Calls that leave nothing behind once the pile is created: a body created and removed,
/// a joint created and removed.
fn noisy_prelude(b: &mut dyn PhysicsBackend) {
    create(b, key(900), Some(RigidBody::dynamic()), Some(ball()), at(3.0, 3.0, 3.0));
    create(b, key(901), None, Some(ball()), at(-3.0, 3.0, 3.0));
    let joint = Joint::new(JointKind::Fixed, key(900), key(901));
    b.create_joint(key(902), &joint, key(900), key(901)).unwrap();
    b.remove_joint(key(902));
    b.remove_body(key(900));
    b.remove_body(key(901));
}

/// Writes that end at the values the pile was created with: a body written, frozen, made
/// kinematic and stripped of its collider, then removed and created again as it was.
fn noisy_writes(b: &mut dyn PhysicsBackend) {
    let body = key(FIRST + 7);
    b.set_transform(body, &at(40.0, 40.0, 40.0));
    b.set_velocity(body, &Velocity { linear: v(1.0, 2.0, 3.0), angular: v(0.0, 1.0, 0.0) });
    b.set_frozen(body, true);
    b.set_body(body, Some(&RigidBody::kinematic()));
    b.set_collider(body, None);
    b.remove_body(body);
    Pile::new(BODIES).create_body(b, FIRST + 7);
}

#[test]
fn the_pile_is_the_same_under_every_pool_size() {
    let reference = pile(1, 0, false);
    assert!(reference.0.iter().any(|seen| !seen.contacts.is_empty()), "the pile collides");
    for threads in [2, 4] {
        let (seen, bytes) = pile(threads, 0, false);
        for (tick, (a, b)) in seen.iter().zip(&reference.0).enumerate() {
            assert_eq!(a, b, "{threads} threads, tick {}", tick + 1);
        }
        assert_eq!(bytes, reference.1, "{threads} threads");
    }
}

#[test]
fn the_pile_is_the_same_whatever_order_built_it() {
    let reference = pile(2, 0, false);
    for (seed, noise) in [(7, false), (0x5eed, true), (99, true)] {
        let (seen, bytes) = pile(2, seed, noise);
        for (tick, (a, b)) in seen.iter().zip(&reference.0).enumerate() {
            assert_eq!(a, b, "seed {seed}, tick {}", tick + 1);
        }
        assert_eq!(bytes, reference.1, "seed {seed}");
    }
}

/// The first key of the distant scene; every key of the pile is below it.
const DISTANT: u64 = 5_000;

/// What the pile's own keys see: its bodies and the events between them.
fn own(seen: Seen) -> Seen {
    let near = |k: lactose_core::StableKey| k < key(DISTANT);
    Seen {
        bodies: seen.bodies.into_iter().filter(|state| near(state.key)).collect(),
        contacts: seen.contacts.into_iter().filter(|c| near(c.target) && near(c.other)).collect(),
        triggers: seen.triggers.into_iter().filter(|t| near(t.target) && near(t.other)).collect(),
    }
}

/// `docs/verification.md`: whether Rapier's single active set makes results depend on
/// unrelated awake bodies. A second pile of balls falling onto its own floor a kilometre
/// away, awake and colliding through the whole run, changes nothing the pile reports.
#[test]
fn distant_awake_bodies_change_nothing_the_pile_reports() {
    for threads in [1, 4] {
        let mut alone = backend(threads);
        Pile::new(BODIES).create(&mut *alone, 0);
        let mut beside = backend(threads);
        create(&mut *beside, key(DISTANT), None, Some(floor()), at(1_000.0, 0.0, 0.0));
        for i in 0..40_u32 {
            let (x, y) = (1_000.0 + f64::from(i % 5) * 0.3, 1.0 + f64::from(i) * 1.1);
            let body = Some(RigidBody::dynamic());
            create(
                &mut *beside,
                key(DISTANT + 1 + u64::from(i)),
                body,
                Some(ball()),
                at(x, y, 0.0),
            );
        }
        Pile::new(BODIES).create(&mut *beside, 0);
        let (alone, beside) = (run(&mut *alone, 1, TICKS), run(&mut *beside, 1, TICKS));
        assert!(
            beside.iter().all(|seen| seen.bodies.iter().any(|state| state.key >= key(DISTANT))),
            "the distant pile stays awake"
        );
        for (tick, (a, b)) in alone.into_iter().zip(beside).enumerate() {
            assert_eq!(a, own(b), "{threads} threads, tick {}", tick + 1);
        }
    }
}
