//! What the snapshot and the step cost, printed and never asserted (`docs/verification.md`:
//! the 100 µs physics snapshot budget for a prediction world, the 3 ms physics phase): the
//! snapshot's size and the time to write and to restore it for 50 bodies beside 0 to 1,000
//! fixed colliders, and the step of 2,000 awake bodies on one and four solver threads. Each
//! time is the median of its repetitions; the machine's load is printed with them. Run it
//! alone, with `--release`, on an idle machine for numbers that mean anything.

#![forbid(unsafe_code)]

mod support;

use std::time::Duration;

use lactose_physics_api::{RigidBody, Shape, Transform};
use support::*;

#[expect(
    clippy::disallowed_types,
    reason = "a measurement outside the step: printed, never asserted"
)]
fn timed(mut run: impl FnMut()) -> Duration {
    let start = std::time::Instant::now();
    run();
    start.elapsed()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// The machine's load averages, as `uptime` prints them.
fn load() -> String {
    std::process::Command::new("uptime")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|error| format!("uptime failed: {error}"))
}

#[test]
fn snapshot_size_and_time_for_fifty_bodies_beside_fixed_colliders() {
    println!("{}", load());
    for statics in [0_u64, 10, 25, 50, 75, 100, 1_000] {
        let mut b = backend(1);
        create(&mut *b, key(1), None, Some(floor()), Transform::IDENTITY);
        for i in 0..50 {
            let x = (i % 10) as f64 * 1.5 - 7.0;
            let z = (i / 10) as f64 * 1.5 - 3.0;
            create(&mut *b, key(10 + i), Some(RigidBody::dynamic()), Some(ball()), at(x, 1.0, z));
        }
        for i in 0..statics {
            let x = (i % 40) as f64 * 3.0 - 60.0;
            let z = (i / 40) as f64 * 3.0 + 10.0;
            let wall = lactose_physics_api::Collider::new(Shape::cuboid(v(1.0, 1.0, 1.0)));
            create(&mut *b, key(10_000 + i), None, Some(wall), at(x, 1.0, z));
        }
        for tick in 1..=10 {
            step(&mut *b, tick, &[]);
        }
        let mut bytes = Vec::new();
        let written = median(
            (0..50)
                .map(|_| {
                    bytes.clear();
                    timed(|| b.snapshot(&mut bytes).unwrap())
                })
                .collect(),
        );
        let mut target = backend(1);
        let restored = median((0..50).map(|_| timed(|| target.restore(&bytes).unwrap())).collect());
        println!(
            "snapshot, 50 bodies + {statics} fixed colliders: {} bytes, write {} µs, restore {} µs",
            bytes.len(),
            written.as_micros(),
            restored.as_micros()
        );
    }
}

#[test]
fn the_step_of_two_thousand_awake_bodies_on_one_and_four_threads() {
    println!("{}", load());
    for threads in [1, 4] {
        let mut b = backend(threads);
        Pile::new(2_000).create(&mut *b, 0);
        let mut awake = 0;
        let samples: Vec<Duration> = (1..=40)
            .map(|tick| {
                let commands = Pile::drive(&mut *b, tick);
                let mut output = None;
                let spent = timed(|| output = Some(step(&mut *b, tick, &commands)));
                awake = output.map_or(0, |out| out.bodies.len());
                spent
            })
            .collect();
        // The first steps build the broad phase and the contact graph: the steady state is
        // the rest.
        let steady = median(samples[10..].to_vec());
        println!(
            "step, 2,000 bodies ({awake} reported at the last), {threads} thread(s): median {} µs, \
             first {} µs",
            steady.as_micros(),
            samples[0].as_micros()
        );
        assert!(awake >= 2_000, "every body is awake: {awake}");
    }
}
