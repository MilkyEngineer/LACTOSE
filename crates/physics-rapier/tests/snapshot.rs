//! Snapshot and restore: the bytes restore into a backend on any pool that writes the same
//! bytes back and steps on exactly as the original does, with the work the calls left for
//! the next step carried; anything that is not such bytes is refused with `InvalidData`
//! and changes nothing.

#![forbid(unsafe_code)]

mod support;

use lactose_core::ErrorKind;
use lactose_physics_api::{Joint, JointKind, PhysicsBackend, RigidBody, Velocity};
use support::*;

const BODIES: u64 = 60;

fn pile(threads: usize, ticks: u32) -> Box<dyn PhysicsBackend> {
    let mut b = backend(threads);
    Pile::new(BODIES).create(&mut *b, 0);
    run(&mut *b, 1, ticks);
    b
}

/// Calls whose work waits for the next step: a creation, a removal, writes, a freeze, a
/// joint created and one removed.
fn pending(b: &mut dyn PhysicsBackend) {
    create(b, key(500), Some(RigidBody::dynamic()), Some(ball()), at(0.0, 9.0, 0.0));
    b.remove_body(key(FIRST + 9));
    b.set_transform(key(FIRST + 10), &at(2.0, 6.0, 2.0));
    let spin = Velocity { linear: v(0.0, 1.0, 0.0), angular: v(0.0, 3.0, 0.0) };
    b.set_velocity(key(FIRST + 11), &spin);
    b.set_frozen(key(FIRST + 12), true);
    b.remove_joint(key(JOINTS[0]));
    let joint = Joint::new(JointKind::Spherical, key(500), key(FIRST + 13));
    b.create_joint(key(1_002), &joint, key(500), key(FIRST + 13)).unwrap();
}

#[test]
fn a_restored_backend_writes_the_same_bytes_and_steps_on_identically() {
    for with_pending in [false, true] {
        let mut original = pile(2, 40);
        if with_pending {
            pending(&mut *original);
        }
        let bytes = snapshot(&*original);
        for threads in [1, 4] {
            let mut restored = backend(threads);
            restored.restore(&bytes).unwrap();
            assert_eq!(snapshot(&*restored), bytes, "the bytes of what they restored");
            let mut again = pile(2, 40);
            if with_pending {
                pending(&mut *again);
            }
            let expected = run(&mut *again, 41, 100);
            let resumed = run(&mut *restored, 41, 100);
            for (tick, (a, b)) in resumed.iter().zip(&expected).enumerate() {
                assert_eq!(a, b, "pending {with_pending}, {threads} threads, tick {}", tick + 41);
            }
            assert_eq!(snapshot(&*restored), snapshot(&*again));
        }
        // And into the backend that took them, back in time.
        let expected = run(&mut *original, 41, 60);
        original.restore(&bytes).unwrap();
        assert_eq!(run(&mut *original, 41, 60), expected);
    }
}

#[test]
fn bytes_that_are_not_a_snapshot_are_refused_and_change_nothing() {
    let mut b = pile(1, 10);
    let bytes = snapshot(&*b);
    let mut other = pile(1, 20);
    let before = snapshot(&*other);
    let mut trailing = bytes.clone();
    trailing.push(0);
    let mut tag = bytes.clone();
    tag[0] ^= 1;
    let mut flipped = bytes.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0xff;
    let refused = [
        Vec::new(),
        bytes[..4].to_vec(),
        bytes[..bytes.len() - 1].to_vec(),
        trailing,
        tag,
        vec![0xff; 64],
    ];
    for garbage in &refused {
        let error = other.restore(garbage).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(snapshot(&*other), before);
    }
    // A flipped byte in the middle decodes to something else or to nothing; either way the
    // backend is whole: it is refused, or it is a world the next snapshot writes back.
    match other.restore(&flipped) {
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::InvalidData);
            assert_eq!(snapshot(&*other), before);
        }
        Ok(()) => assert_eq!(snapshot(&*other), flipped),
    }
    b.restore(&bytes).unwrap();
}

#[test]
fn the_snapshot_grows_with_the_world_and_is_a_function_of_it() {
    let small = snapshot(&*pile(1, 5));
    let empty = snapshot(&*backend(1));
    assert!(empty.len() < small.len());
    assert_eq!(&small[..4], b"LPR1");
    // Two snapshots of one state are equal, and so are two backends' after equal runs.
    let b = pile(4, 5);
    assert_eq!(snapshot(&*b), snapshot(&*b));
    assert_eq!(snapshot(&*b), small);
}
