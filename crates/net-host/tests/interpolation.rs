//! The interpolation buffer (README, "The interpolation buffer"): the sampling cases,
//! bracketed, held at either end and around a teleport, missing; the ring's keep-newest
//! rule, independent of arrival order; slot allocation and staleness; and a stress test of
//! the sequence lock, one writer and three readers, in which every sample must be a state
//! that was pushed, checked by words derived from one counter. `interpolation_alloc.rs`
//! counts allocations in a binary of its own.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use lactose_core::{ErrorKind, Tick};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::InterpolationSlot;
use lactose_net_host::InterpolationBuffer;
use lactose_net_host::interpolation::{
    InterpolationReader, InterpolationWriter, PresentationTime, ReceivedState, RemoteSample,
    STATES_PER_SLOT, SampleStatus,
};

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn state(tick: u32, x: f64) -> ReceivedState {
    ReceivedState {
        tick: Tick(tick),
        teleported: Tick(0),
        position: Vec3::new(s(x), s(-x), s(0.5)),
        rotation: Quat::from_rotation_z(s(x / 100.0)),
    }
}

fn at(tick: u32, fraction: u16) -> PresentationTime {
    PresentationTime { tick: Tick(tick), fraction }
}

fn held(state: &ReceivedState) -> RemoteSample {
    RemoteSample { status: SampleStatus::Held, position: state.position, rotation: state.rotation }
}

fn buffer(capacity: u32) -> (InterpolationWriter, InterpolationReader) {
    InterpolationBuffer::split(capacity)
}

#[test]
fn two_states_around_the_time_interpolate() {
    let (mut writer, reader) = buffer(4);
    let slot = writer.allocate().unwrap();
    let (earlier, later) = (state(10, 0.0), state(14, 4.0));
    writer.push(slot, earlier).unwrap();
    writer.push(slot, later).unwrap();
    let cases =
        [(at(12, 0), 0.5), (at(11, 32_768), 0.375), (at(10, 0), 0.0), (at(13, 65_535), 1.0)];
    for (time, expected_t) in cases {
        let sample = reader.sample(slot, time);
        assert_eq!(sample.status, SampleStatus::Interpolated, "{time:?}");
        let t = (Scalar::from_u32(time.tick.0 - 10)
            + Scalar::from_u32(u32::from(time.fraction)) / Scalar::from_u32(65_536))
            / Scalar::from_u32(4);
        if expected_t != 1.0 {
            assert_eq!(t, s(expected_t));
        }
        assert_eq!(sample.position, earlier.position.lerp(later.position, t), "{time:?}");
        assert_eq!(sample.rotation, earlier.rotation.nlerp(later.rotation, t), "{time:?}");
    }
    assert_eq!(reader.sample(slot, at(12, 0)).position.x, s(2.0));
}

#[test]
fn outside_the_states_the_sample_holds_the_nearest() {
    let (mut writer, reader) = buffer(4);
    let slot = writer.allocate().unwrap();
    let states = [state(10, 0.0), state(14, 4.0), state(16, 6.0)];
    for state in states {
        writer.push(slot, state).unwrap();
    }
    assert_eq!(reader.sample(slot, at(16, 0)), held(&states[2]), "at the newest");
    assert_eq!(reader.sample(slot, at(40, 9)), held(&states[2]), "past the newest");
    assert_eq!(reader.sample(slot, at(9, 65_535)), held(&states[0]), "before the oldest");
    assert_eq!(reader.sample(slot, at(15, 0)).status, SampleStatus::Interpolated);
    // One state: held on either side.
    let lone = writer.allocate().unwrap();
    writer.push(lone, states[1]).unwrap();
    assert_eq!(reader.sample(lone, at(2, 0)), held(&states[1]));
    assert_eq!(reader.sample(lone, at(14, 0)), held(&states[1]));
    assert_eq!(reader.sample(lone, at(99, 0)), held(&states[1]));
}

#[test]
fn nothing_blends_across_a_teleport() {
    let (mut writer, reader) = buffer(8);
    let earlier = state(10, 0.0);
    let teleport = |teleported: u32| {
        let mut later = state(14, 100.0);
        later.teleported = Tick(teleported);
        later
    };
    // Teleported at 12: the earlier state until 12, the later from 12 on.
    let slot = writer.allocate().unwrap();
    writer.push(slot, earlier).unwrap();
    writer.push(slot, teleport(12)).unwrap();
    assert_eq!(reader.sample(slot, at(10, 0)), held(&earlier));
    assert_eq!(reader.sample(slot, at(11, 65_535)), held(&earlier));
    assert_eq!(reader.sample(slot, at(12, 0)), held(&teleport(12)));
    assert_eq!(reader.sample(slot, at(13, 40_000)), held(&teleport(12)));
    // At the later state's own tick: the earlier until then.
    let slot = writer.allocate().unwrap();
    writer.push(slot, teleport(14)).unwrap();
    writer.push(slot, earlier).unwrap();
    assert_eq!(reader.sample(slot, at(13, 65_535)), held(&earlier), "pushed out of order");
    // A teleport at or before the earlier state is behind both: they blend.
    for teleported in [10, 3] {
        let slot = writer.allocate().unwrap();
        writer.push(slot, earlier).unwrap();
        writer.push(slot, teleport(teleported)).unwrap();
        assert_eq!(reader.sample(slot, at(12, 0)).status, SampleStatus::Interpolated);
        assert_eq!(reader.sample(slot, at(12, 0)).position.x, s(50.0));
    }
}

#[test]
fn a_slot_with_nothing_or_a_stale_generation_is_missing() {
    let (mut writer, reader) = buffer(2);
    let first = writer.allocate().unwrap();
    assert_eq!(first, InterpolationSlot { index: 0, generation: 1 });
    assert_eq!(reader.sample(first, at(1, 0)), RemoteSample::MISSING, "no state yet");
    writer.push(first, state(1, 1.0)).unwrap();
    assert_eq!(reader.sample(first, at(1, 0)).status, SampleStatus::Held);

    writer.release(first).unwrap();
    assert_eq!(reader.sample(first, at(1, 0)), RemoteSample::MISSING, "released");
    assert_eq!(writer.push(first, state(2, 2.0)).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(writer.release(first).unwrap_err().kind(), ErrorKind::NotFound);

    let second = writer.allocate().unwrap();
    assert_eq!(second, InterpolationSlot { index: 0, generation: 2 }, "the released slot");
    writer.push(second, state(3, 3.0)).unwrap();
    assert_eq!(reader.sample(first, at(3, 0)), RemoteSample::MISSING, "the old generation");
    assert_eq!(reader.sample(second, at(3, 0)).status, SampleStatus::Held);

    let third = writer.allocate().unwrap();
    assert_eq!(third, InterpolationSlot { index: 1, generation: 1 });
    assert_eq!(writer.allocate().unwrap_err().kind(), ErrorKind::ResourceExhausted);
    writer.release(third).unwrap();
    writer.release(second).unwrap();
    assert_eq!(writer.allocate().unwrap(), InterpolationSlot { index: 0, generation: 3 });
    assert_eq!(writer.allocate().unwrap(), InterpolationSlot { index: 1, generation: 2 });

    for bogus in [
        InterpolationSlot { index: 9, generation: 1 },
        InterpolationSlot { index: 0, generation: 0 },
    ] {
        assert_eq!(reader.sample(bogus, at(3, 0)), RemoteSample::MISSING);
        assert_eq!(writer.push(bogus, state(3, 3.0)).unwrap_err().kind(), ErrorKind::NotFound);
    }
}

/// Every sample of `slot` at a grid of times, for comparing two rings.
fn samples(reader: &InterpolationReader, slot: InterpolationSlot) -> Vec<RemoteSample> {
    (0..40)
        .flat_map(|tick| [0, 1, 32_768, 65_535].map(|f| reader.sample(slot, at(tick, f))))
        .collect()
}

#[test]
fn the_ring_keeps_the_newest_states_whatever_the_arrival_order() {
    let (mut writer, reader) = buffer(3);
    let ticks: Vec<u32> = (0..12).map(|n| 3 * n + 1).collect();
    let in_order = writer.allocate().unwrap();
    for &tick in &ticks {
        writer.push(in_order, state(tick, f64::from(tick))).unwrap();
    }
    // The same states in another order, older ones arriving after the ring filled.
    let shuffled = writer.allocate().unwrap();
    for &index in &[11_usize, 3, 7, 0, 10, 4, 9, 1, 8, 2, 6, 5] {
        writer.push(shuffled, state(ticks[index], f64::from(ticks[index]))).unwrap();
    }
    assert_eq!(samples(&reader, in_order), samples(&reader, shuffled));
    // The eight newest are kept: 13 to 34; the oldest held is 13.
    let oldest = ticks[ticks.len() - STATES_PER_SLOT];
    assert_eq!(reader.sample(in_order, at(0, 0)), held(&state(oldest, f64::from(oldest))));
    // Older than the oldest kept: dropped. An equal tick: replaced.
    writer.push(in_order, state(2, 999.0)).unwrap();
    assert_eq!(reader.sample(in_order, at(0, 0)), held(&state(oldest, f64::from(oldest))));
    writer.push(in_order, state(34, -5.0)).unwrap();
    assert_eq!(reader.sample(in_order, at(50, 0)), held(&state(34, -5.0)));
    writer.push(in_order, state(oldest, 7.0)).unwrap();
    assert_eq!(reader.sample(in_order, at(0, 0)), held(&state(oldest, 7.0)));
    // A newer state than every kept one evicts the oldest.
    writer.push(in_order, state(35, 35.0)).unwrap();
    let next = ticks[ticks.len() - STATES_PER_SLOT + 1];
    assert_eq!(reader.sample(in_order, at(0, 0)), held(&state(next, f64::from(next))));
}

#[test]
fn a_batch_is_the_sample_of_each_slot() {
    let (mut writer, reader) = buffer(8);
    let slots: Vec<InterpolationSlot> = (0..6).map(|_| writer.allocate().unwrap()).collect();
    for (n, &slot) in slots.iter().enumerate() {
        for tick in 0..u32::try_from(n).unwrap() {
            writer.push(slot, state(tick * 2, f64::from(tick) + n as f64)).unwrap();
        }
    }
    writer.release(slots[4]).unwrap();
    let mut batch = vec![RemoteSample::MISSING; slots.len() + 2];
    for time in [at(0, 0), at(3, 100), at(9, 65_535)] {
        reader.sample_batch(&slots, time, &mut batch);
        for (index, &slot) in slots.iter().enumerate() {
            assert_eq!(batch[index], reader.sample(slot, time));
        }
        assert_eq!(batch[slots.len()..], [RemoteSample::MISSING; 2], "the tail is untouched");
    }
    assert_eq!(batch[4], RemoteSample::MISSING);
}

// ---- The sequence lock under contention ----

/// The state pushed at `counter`: every word a function of the counter, so a sample made
/// of two states' words is recognised.
fn checked(counter: u64) -> ReceivedState {
    let c = i64::try_from(counter).expect("small");
    let word = |k: i64| Scalar::from_i64(c.wrapping_mul(k) % 1_000_003);
    ReceivedState {
        tick: Tick(u32::try_from(counter).expect("small")),
        teleported: Tick(0),
        position: Vec3::new(Scalar::from_i64(c), word(7), word(13)),
        rotation: Quat::new(word(17), word(19), word(23), word(29)),
    }
}

/// Whether a held sample is exactly one pushed state.
fn is_pushed(sample: &RemoteSample) -> bool {
    let counter = sample.position.x.to_i64();
    let Ok(counter) = u64::try_from(counter) else { return false };
    let state = checked(counter);
    sample.position == state.position && sample.rotation == state.rotation
}

#[test]
fn every_sample_under_contention_is_a_pushed_state() {
    const SLOTS: u32 = 4;
    const PUSHES: u64 = 400_000;
    const READERS: usize = 3;
    let (mut writer, reader) = buffer(SLOTS);
    let mut slots: Vec<InterpolationSlot> =
        (0..SLOTS).map(|_| writer.allocate().unwrap()).collect();
    let published: Arc<Vec<AtomicU64>> =
        Arc::new((0..SLOTS).map(|_| AtomicU64::new(u64::MAX)).collect());
    let done = Arc::new(AtomicBool::new(false));
    let checked_samples = Arc::new(AtomicU64::new(0));
    let readers: Vec<_> = (0..READERS)
        .map(|n| {
            let reader = reader.clone();
            let published = Arc::clone(&published);
            let done = Arc::clone(&done);
            let checked_samples = Arc::clone(&checked_samples);
            std::thread::spawn(move || {
                let mut torn = 0_u64;
                let mut count = 0_u64;
                let mut batch = [RemoteSample::MISSING; SLOTS as usize];
                let mut slots = vec![InterpolationSlot { index: 0, generation: 0 }; SLOTS as usize];
                while !done.load(Ordering::Relaxed) {
                    for (index, slot) in slots.iter_mut().enumerate() {
                        let word = published[index].load(Ordering::Acquire);
                        if word != u64::MAX {
                            *slot = InterpolationSlot {
                                index: u32::try_from(index).unwrap(),
                                generation: u32::try_from(word).unwrap(),
                            };
                        }
                    }
                    // Newest, oldest, and the batch path, alternately.
                    let time =
                        if count.is_multiple_of(2) { at(u32::MAX >> 2, 0) } else { at(0, 0) };
                    if n == 0 {
                        reader.sample_batch(&slots, time, &mut batch);
                    } else {
                        for (sample, &slot) in batch.iter_mut().zip(&slots) {
                            *sample = reader.sample(slot, time);
                        }
                    }
                    for sample in &batch {
                        match sample.status {
                            SampleStatus::Held => {
                                count += 1;
                                torn += u64::from(!is_pushed(sample));
                            }
                            SampleStatus::Missing => {}
                            SampleStatus::Interpolated => torn += 1,
                        }
                    }
                }
                checked_samples.fetch_add(count, Ordering::Relaxed);
                torn
            })
        })
        .collect();
    for counter in 0..PUSHES {
        let index = (counter % u64::from(SLOTS)) as usize;
        if counter % 5_003 == 0 {
            // Reallocation under the readers: the old generation reads as missing.
            writer.release(slots[index]).unwrap();
            slots[index] = writer.allocate().unwrap();
        }
        published[index].store(u64::from(slots[index].generation), Ordering::Release);
        writer.push(slots[index], checked(counter)).unwrap();
    }
    done.store(true, Ordering::Relaxed);
    let torn: u64 = readers.into_iter().map(|reader| reader.join().unwrap()).sum();
    let samples = checked_samples.load(Ordering::Relaxed);
    println!("{samples} samples checked, {torn} torn, {PUSHES} pushes");
    assert_eq!(torn, 0);
    assert!(samples > 0);
}
