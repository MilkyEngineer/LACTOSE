//! The latest-sample slot (README, "The latest sample"): what `read` returns before and
//! after a `publish`, and the stress test of reads against one and two publishers, every
//! sample carrying a checksum word so a read that mixed two publishes is seen. The loom
//! models of the same protocol are `src/models.rs`.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use lactose_input::{Action, Actions};
use lactose_input_host::{LatestSample, RawSample};

/// The sample's six words, so NaN payloads compare.
fn words(sample: RawSample) -> [u32; 6] {
    [
        sample.movement_x.to_bits(),
        sample.movement_y.to_bits(),
        sample.yaw.to_bits(),
        sample.pitch.to_bits(),
        sample.held.bits(),
        sample.latched.bits(),
    ]
}

#[test]
fn read_before_the_first_publish_is_idle() {
    assert_eq!(words(LatestSample::new().read()), words(RawSample::IDLE));
    assert_eq!(words(LatestSample::default().read()), words(RawSample::IDLE));
    static UNPUBLISHED: LatestSample = LatestSample::new();
    assert_eq!(words(UNPUBLISHED.read()), words(RawSample::IDLE));
}

#[test]
fn read_after_publish_returns_the_published_sample() {
    let slot = LatestSample::new();
    let sample = RawSample {
        movement_x: -0.25,
        movement_y: f32::from_bits(0x7fc0_1234),
        yaw: -0.0,
        pitch: f32::NEG_INFINITY,
        held: Actions::EMPTY.with(Action::Jump).with_bit(31),
        latched: Actions::from_bits(u32::MAX),
    };
    slot.publish(sample);
    assert_eq!(words(slot.read()), words(sample));
    assert_eq!(words(slot.read()), words(sample), "a read does not consume");
    let next = RawSample { yaw: 1.5, ..RawSample::IDLE };
    slot.publish(next);
    assert_eq!(words(slot.read()), words(next));
    slot.publish(RawSample::IDLE);
    assert_eq!(words(slot.read()), words(RawSample::IDLE));
}

#[test]
fn a_static_slot_is_shared_between_threads() {
    static LATEST: LatestSample = LatestSample::new();
    let sample = RawSample { yaw: 0.5, pitch: -0.5, ..RawSample::IDLE };
    thread::spawn(move || LATEST.publish(sample)).join().unwrap();
    assert_eq!(words(LATEST.read()), words(sample));
}

// ---- Stress ----

/// The checksum of a sample's first five words, stored in the sixth.
fn checksum(words: [u32; 5]) -> u32 {
    words
        .iter()
        .fold(0x9e37_79b9u32, |sum, &word| (sum ^ word).wrapping_mul(0x0100_0193).rotate_left(5))
}

/// Publish `k` of `publisher` (1 or 2): the publisher in `movement_x`, the count in `held`,
/// words mixed from both in between, and the checksum in `latched`. The floats are finite
/// (in `[1, 2)`), so no NaN payload is at stake here.
fn stamped(publisher: u32, k: u32) -> RawSample {
    let mix = |salt: u32| {
        let bits = k.wrapping_mul(0x2545_f491).rotate_left(salt) ^ publisher.wrapping_mul(salt);
        f32::from_bits(0x3f80_0000 | (bits & 0x007f_ffff))
    };
    let mut sample = RawSample {
        movement_x: publisher as f32,
        movement_y: mix(7),
        yaw: mix(13),
        pitch: mix(29),
        held: Actions::from_bits(k),
        latched: Actions::EMPTY,
    };
    let [a, b, c, d, e, _] = words(sample);
    sample.latched = Actions::from_bits(checksum([a, b, c, d, e]));
    sample
}

/// The publisher and count of a whole sample; panics on a torn one.
fn unstamp(sample: RawSample) -> (u32, u32) {
    let [a, b, c, d, e, sum] = words(sample);
    assert_eq!(sum, checksum([a, b, c, d, e]), "torn read: {sample:?}");
    let publisher = sample.movement_x as u32;
    let k = sample.held.bits();
    assert_eq!(words(sample), words(stamped(publisher, k)), "torn read: {sample:?}");
    (publisher, k)
}

/// `publishers` threads each publish `count` stamped samples while `readers` threads read
/// until they are done. Every read is whole and, per publisher, never older than the one
/// the same reader saw before it; after the joins the slot holds some publisher's last
/// sample. Returns the reads per reader.
fn stress(publishers: u32, readers: usize, count: u32) -> Vec<u64> {
    let slot = Arc::new(LatestSample::new());
    // Something to read from the start, so no read is `IDLE`.
    slot.publish(stamped(1, 0));
    let done = Arc::new(AtomicBool::new(false));
    let reading: Vec<_> = (0..readers)
        .map(|_| {
            let (slot, done) = (Arc::clone(&slot), Arc::clone(&done));
            thread::spawn(move || {
                let mut newest = [0u32; 3];
                let mut reads = 0u64;
                loop {
                    let finished = done.load(Ordering::Acquire);
                    let (publisher, k) = unstamp(slot.read());
                    let seen = &mut newest[publisher as usize];
                    assert!(k >= *seen, "publisher {publisher}: read {k} after {seen}");
                    *seen = k;
                    reads += 1;
                    if finished {
                        return reads;
                    }
                }
            })
        })
        .collect();
    let publishing: Vec<_> = (1..=publishers)
        .map(|publisher| {
            let slot = Arc::clone(&slot);
            thread::spawn(move || {
                for k in 1..=count {
                    slot.publish(stamped(publisher, k));
                }
            })
        })
        .collect();
    for publisher in publishing {
        publisher.join().unwrap();
    }
    done.store(true, Ordering::Release);
    let reads = reading.into_iter().map(|reader| reader.join().unwrap()).collect();
    let (publisher, k) = unstamp(slot.read());
    assert!((1..=publishers).contains(&publisher) && k == count, "last {publisher} {k}");
    reads
}

fn publishes() -> u32 {
    if cfg!(miri) { 50 } else { 200_000 }
}

#[test]
fn reads_against_one_publisher_are_whole() {
    let reads = stress(1, 3, publishes());
    assert!(reads.iter().all(|&n| n >= 1), "{reads:?}");
}

#[test]
fn reads_against_two_publishers_are_whole() {
    let reads = stress(2, 3, publishes());
    assert!(reads.iter().all(|&n| n >= 1), "{reads:?}");
}
