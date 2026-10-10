//! The step and the snapshot bytes are functions of the row history (every create, remove,
//! write and command, in order), not of the pool or of the order of the calls between two
//! steps: a pile stepped on one, two and four solver threads, and built in different orders
//! with writes, removals and re-creations between two steps that end at the same rows,
//! reports the same bodies and events every tick and writes the same bytes. They are not
//! functions of a body's own rows: what the pile reports depends on every body the world
//! holds or held, far-away awake bodies that touch only each other included, and on bodies
//! stepped once and removed. Bit equality holds only between worlds whose row histories are
//! identical.

#![forbid(unsafe_code)]

mod support;

use lactose_core::StableKey;
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

/// How long the far scenes run beside the pile: long enough for them to reach it.
const LONG: u32 = 400;

/// What the pile's own keys see: its bodies and the events between them.
fn own(seen: Seen) -> Seen {
    let near = |k: StableKey| k < key(DISTANT);
    Seen {
        bodies: seen.bodies.into_iter().filter(|state| near(state.key)).collect(),
        contacts: seen.contacts.into_iter().filter(|c| near(c.target) && near(c.other)).collect(),
        triggers: seen.triggers.into_iter().filter(|t| near(t.target) && near(t.other)).collect(),
    }
}

/// The first tick whose reports differ, counting from one.
fn first_difference(a: &[Seen], b: &[Seen]) -> Option<u32> {
    let index = a.iter().zip(b).position(|(a, b)| a != b);
    index.map(|i| u32::try_from(i).unwrap() + 1)
}

/// `docs/verification.md` and review N1: Rapier's shared structures make a body's results
/// depend on unrelated awake bodies. A second pile of balls falling onto its own floor a
/// kilometre away, colliding only with itself and never touched by a call after it is
/// created, changes what the pile reports; so do far balls teleported onto their floors
/// every 25 ticks, keyed above or below the pile. This test records the dependence; the
/// ticks in the comments were observed, not asserted.
#[test]
fn distant_awake_bodies_change_what_the_pile_reports() {
    let above: Vec<u64> = (DISTANT..DISTANT + 7).collect();
    let below = [2, 3, 4, 6, 7, 8, 9];
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
        let (alone, beside) = (run(&mut *alone, 1, LONG), run(&mut *beside, 1, LONG));
        assert!(
            beside[..TICKS as usize]
                .iter()
                .all(|seen| seen.bodies.iter().any(|state| state.key >= key(DISTANT))),
            "the distant pile is awake"
        );
        let beside: Vec<Seen> = beside.into_iter().map(own).collect();
        // Observed: first differs at tick 238 on one and four threads.
        let differs = first_difference(&alone, &beside);
        assert!(differs.is_some(), "{threads} threads: the distant pile changed nothing");

        let alone = history(threads, &[], Far::Kept, LONG);
        for far in [&above[..], &below[..]] {
            // Observed: first differs at tick 274, above and below, on one and
            // four threads.
            let teleported = history(threads, far, Far::Teleported, LONG);
            let differs = first_difference(&alone, &teleported);
            assert!(differs.is_some(), "{threads} threads, {far:?}: teleporting changed nothing");
        }
    }
}

/// The tick [`history`] drops ten balls into the pile, which take whatever slots the
/// arenas' free lists offer.
const LATE: u32 = 40;

/// What [`history`] does with its far scene after creating it.
#[derive(Clone, Copy)]
enum Far {
    /// Nothing.
    Kept,
    /// Removes it after the tick.
    Removed(u32),
    /// Every 25 ticks, places its balls one to three units above the floors.
    Teleported,
}

/// The pile on `threads` threads for `ticks` ticks, with an unrelated scene of `far` keys a
/// kilometre away (fixed colliders and dynamic balls in turn) created with it and treated
/// as `what` says; ten balls join the pile at [`LATE`]. Each tick's report without the far
/// scene.
fn history(threads: usize, far: &[u64], what: Far, ticks: u32) -> Vec<Seen> {
    let x = |i: usize| 1_000.0 + 3.0 * f64::from(u32::try_from(i).unwrap());
    let mut b = backend(threads);
    for (i, &n) in far.iter().enumerate() {
        if i % 2 == 0 {
            create(&mut *b, key(n), None, Some(floor()), at(x(i), 0.0, 500.0));
        } else {
            create(&mut *b, key(n), Some(RigidBody::dynamic()), Some(ball()), at(x(i), 3.0, 500.0));
        }
    }
    Pile::new(BODIES).create(&mut *b, 0);
    let near = |k: StableKey| !far.iter().any(|&n| key(n) == k);
    (1..=ticks)
        .map(|tick| {
            match what {
                Far::Removed(after) if tick == after + 1 => {
                    for &n in far {
                        b.remove_body(key(n));
                    }
                }
                Far::Teleported if tick % 25 == 0 => {
                    for (i, &n) in far.iter().enumerate().filter(|(i, _)| i % 2 == 1) {
                        let up = 1.0 + f64::from((tick / 25 + u32::try_from(i).unwrap()) % 3);
                        b.set_transform(key(n), &at(x(i), up, 500.0));
                    }
                }
                _ => {}
            }
            if tick == LATE {
                for n in 0..10_u32 {
                    let (x, y) = (-2.0 + 0.4 * f64::from(n), 6.0 + f64::from(n));
                    let body = Some(RigidBody::dynamic());
                    create(&mut *b, key(400 + u64::from(n)), body, Some(ball()), at(x, y, 0.3));
                }
            }
            let commands = Pile::drive(&mut *b, tick);
            let seen = Seen::from(&step(&mut *b, tick, &commands));
            Seen {
                bodies: seen.bodies.into_iter().filter(|state| near(state.key)).collect(),
                contacts: seen
                    .contacts
                    .into_iter()
                    .filter(|c| near(c.target) && near(c.other))
                    .collect(),
                triggers: seen
                    .triggers
                    .into_iter()
                    .filter(|t| near(t.target) && near(t.other))
                    .collect(),
            }
        })
        .collect()
}

/// Review S2, `docs/verification.md`: results depend on the arenas' history. Unrelated
/// bodies stepped once and removed leave free slots that the late balls take ahead of
/// where they would be: the pile is equal up to then and differs after. The control keeps
/// the same scene, which within these 120 ticks happens not to reach the pile (over longer
/// runs it does: `distant_awake_bodies_change_what_the_pile_reports`).
#[test]
fn what_the_pile_reports_depends_on_the_arena_history() {
    let above: Vec<u64> = (DISTANT..DISTANT + 7).collect();
    let below = [2, 3, 4, 6, 7, 8, 9];
    assert!(below.iter().all(|&n| n < FIRST && n != FLOOR && n != PADDLE));
    for threads in [1, 4] {
        let alone = history(threads, &[], Far::Kept, TICKS);
        let kept = history(threads, &above, Far::Kept, TICKS);
        assert_eq!(first_difference(&alone, &kept), None, "{threads} threads, the control");
        for far in [&above[..], &below[..]] {
            let removed = first_difference(&alone, &history(threads, far, Far::Removed(1), TICKS));
            assert!(
                removed.is_some_and(|tick| tick >= LATE),
                "{threads} threads, {far:?} removed: {removed:?}"
            );
        }
    }
}
