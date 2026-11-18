//! What the remote sample and upload cost for 5,000 remote rows (`docs/verification.md`, the
//! row "Remote transform upload"; budget row `remote-upload-5k`), as steering: the schedule's
//! frame on a client world's snapshot, which samples every remote row in one batch call and
//! converts it, then the upload from the frame's job, with the network thread pushing a state
//! into every slot at 30 Hz beside it. Frames are paced at 144 Hz for ten seconds, and the
//! median and the 99th percentile are printed, never asserted, with those of the batch call
//! alone, made a second time after the frame on the same slots (not the batch call's share of
//! the frame's time):
//!
//! `cargo nextest run -p lactose-presentation --run-ignored only --cargo-profile release
//! --no-capture -E 'test(/cost::/)'`
//!
//! The renderer's `RemoteWriter::write` is not built yet, so the upload writes to a closure
//! that counts the rows: the ring's copy is not in the figure. The median is bimodal on the
//! M1: paced on an otherwise quiet machine the sample took about 1.8 ms, with a core kept busy
//! beside it or unpaced about 0.3 ms, and the cause is not measured. The network thread here
//! is a plain thread. `measure-m2` makes the recorded measurement: it runs the writer under
//! `ThreadClass::Network`, times the batch call inside the frame, and reports the reader's
//! retry rate.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::common;

use common::{Ignored, Scene, in_job, key, mesh};
use lactose_core::Tick;
use lactose_ecs::WorldKind;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_api::{InterpolationSlot, RemoteSlot};
use lactose_net_host::InterpolationBuffer;
use lactose_net_host::interpolation::{PresentationTime, ReceivedState, RemoteSample};
use lactose_present_api::snapshot_channel;
use lactose_presentation::{PresentView, RemoteView, upload};

const REMOTE: u32 = 5_000;
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 144);
const FRAMES: usize = 1_440;
const SEND_HZ: f64 = 30.0;

/// The state the network thread pushes for slot `n` at server tick `tick`.
fn state(tick: u32, n: u32) -> ReceivedState {
    let x = Scalar::from_f64(f64::from(n) + f64::from(tick) * 0.1);
    let position = Vec3::new(x, Scalar::ONE, Scalar::from_u32(n % 64));
    ReceivedState { tick: Tick(tick), teleported: Tick(0), position, rotation: Quat::IDENTITY }
}

/// What one frame took: the schedule's frame and the upload; and, after them, the batch call
/// alone on the same slots, the interpolation buffer's share of the frame.
#[derive(Clone, Copy, Default)]
struct Timing {
    sample: Duration,
    upload: Duration,
    batch: Duration,
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn report(label: &str, mut times: Vec<Duration>) {
    times.sort_unstable();
    let (median, p99) = (percentile(&times, 0.5), percentile(&times, 0.99));
    println!(
        "remote-upload-5k {label}: median {:.1} us, p99 {:.1} us, max {:.1} us",
        median.as_secs_f64() * 1e6,
        p99.as_secs_f64() * 1e6,
        times[times.len() - 1].as_secs_f64() * 1e6,
    );
}

#[test]
#[ignore = "a measurement, run by hand in release: timings are reported"]
fn remote_upload_of_5k_rows() {
    let mut scene = Scene::new(WorldKind::Client, Tick(1), 4);
    let (mut writer, reader) = InterpolationBuffer::split(REMOTE);
    let mut slots = Vec::new();
    for n in 1..=REMOTE {
        let slot = writer.allocate().unwrap();
        slots.push(slot);
        writer.push(slot, state(1, n)).unwrap();
        let row = (RemoteSlot { slot }, mesh(u64::from(n)));
        scene.world_mut().spawn(key(u64::from(n)), common::REGION, row).unwrap();
    }
    scene.publish();
    let executor = scene.executor.clone();
    // The frame's half: the snapshot's reader, which acquires the same pair every frame since
    // nothing is published again, and the schedule.
    let snapshot = std::mem::replace(&mut scene.reader, snapshot_channel().1);
    let presentation = std::mem::take(&mut scene.presentation);

    // The network thread: a state for every slot at 30 Hz.
    let begun = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let slots: Arc<[InterpolationSlot]> = slots.into();
    let network = {
        let (stop, slots) = (Arc::clone(&stop), Arc::clone(&slots));
        std::thread::spawn(move || {
            let mut tick = 1;
            while !stop.load(Ordering::Relaxed) {
                tick += 1;
                let due = begun + Duration::from_secs_f64(f64::from(tick - 1) / SEND_HZ);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                for (n, &slot) in (1..).zip(slots.iter()) {
                    writer.push(slot, state(tick, n)).unwrap();
                }
            }
        })
    };

    let shared = Arc::new(Mutex::new((snapshot, presentation, reader)));
    let written = Arc::new(AtomicUsize::new(0));
    let mut timings = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let due = begun + FRAME * frame as u32;
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
        // Two send intervals behind the newest state, as the clock estimate would place it.
        let server = (begun.elapsed().as_secs_f64() * SEND_HZ - 2.0).max(1.0);
        let at = PresentationTime {
            tick: Tick(server as u32),
            fraction: (server.fract() * 65536.0) as u16,
        };
        let timing = Arc::new(Mutex::new(Timing::default()));
        let (shared, written, timing_out, slots) =
            (Arc::clone(&shared), Arc::clone(&written), Arc::clone(&timing), Arc::clone(&slots));
        in_job(&executor, move |ctx| {
            let mut held = shared.lock().expect("unpoisoned");
            let (snapshot, presentation, reader) = &mut *held;
            let pair = snapshot.acquire().expect("published");
            let started = Instant::now();
            let remote = RemoteView { buffer: reader, at };
            let view = PresentView { fraction: 0.5, focus: None, remote: Some(remote) };
            let shown = presentation.frame(&pair, &view, &mut Ignored);
            let sampled = started.elapsed();
            assert_eq!(presentation.remote().len() + shown.missing, REMOTE as usize);
            let started = Instant::now();
            upload(ctx, presentation.remote(), |batch| {
                written.fetch_add(batch.len(), Ordering::Relaxed);
            });
            let uploaded = started.elapsed();
            let mut samples = vec![RemoteSample::MISSING; slots.len()];
            let started = Instant::now();
            reader.sample_batch(&slots, at, &mut samples);
            let batch = started.elapsed();
            *timing_out.lock().expect("unpoisoned") =
                Timing { sample: sampled, upload: uploaded, batch };
        });
        timings.push(*timing.lock().expect("unpoisoned"));
    }
    stop.store(true, Ordering::Relaxed);
    network.join().expect("the network thread");
    assert_eq!(written.load(Ordering::Relaxed), FRAMES * REMOTE as usize);

    println!("remote-upload-5k: {FRAMES} frames at 144 Hz, {REMOTE} remote rows, 4 frame workers");
    report("sample", timings.iter().map(|t| t.sample).collect());
    report("upload", timings.iter().map(|t| t.upload).collect());
    report("frame", timings.iter().map(|t| t.sample + t.upload).collect());
    report(
        "batch call alone, a second call after the frame",
        timings.iter().map(|t| t.batch).collect(),
    );
}
