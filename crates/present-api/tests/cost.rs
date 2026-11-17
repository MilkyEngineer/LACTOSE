//! What extract's pass costs (`README.md`, "Proof for milestone 2", Cost): a server world of
//! instances, half of which move every tick, a stated share of them presented rows the
//! snapshot holds, extracted to a null sink that only counts. Asserted on what does not
//! depend on the machine (how many rows each pass writes, and the snapshot's rows); the
//! times are measured by an ignored test, run by hand in release, and reported, never
//! asserted:
//!
//! `cargo nextest run -p lactose-present-api --run-ignored only --cargo-profile release
//! --no-capture -E 'test(/cost::/)'`
//!
//! The device's ring is the renderer's and is not built yet: this file measures the null
//! sink alone.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[expect(
    clippy::disallowed_types,
    reason = "a measurement, run by hand: timings are reported"
)]
use std::time::{Duration, Instant};

use crate::common;

use common::{REGION, Rig, Shown, Walker, at, key, mesh, scalar, simulation};
use lactose_core::{StableKey, Tick};
use lactose_ecs::{LayoutConfig, WorldKind};
use lactose_math::Scalar;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, extract_state};

/// A sink that keeps nothing: it counts the rows written, which a relaxed add per batch does
/// at no measurable cost.
#[derive(Clone, Default)]
struct NullSink(Arc<AtomicUsize>);

impl InstanceSink for NullSink {
    fn begin(&mut self, _pass: ExtractPass) {}

    fn removed(&mut self, _keys: &[StableKey]) {}

    fn write(&self, rows: &[InstanceRow]) {
        self.0.fetch_add(rows.len(), Ordering::Relaxed);
    }

    fn end(&mut self) {}
}

/// A server world of `rows` instances, the first `moving` of which walk one unit a tick and
/// the first `presented` of which carry `Shown`, with a null sink and, if `snapshot`, a
/// snapshot channel. Returns the rig, the sink's count and the rows each pass writes.
fn scene(rows: u64, moving: u64, presented: u64, snapshot: bool, workers: u32) -> (Rig, NullSink) {
    let mut rig = Rig::new(
        simulation(WorldKind::Server, LayoutConfig::default(), Tick(1)),
        common::executor(workers),
    );
    let sink = NullSink::default();
    let state = extract_state(rig.simulation_mut()).expect("setup ran");
    drop(state.set_sink(Some(Box::new(sink.clone()))));
    if !snapshot {
        drop(state.set_snapshot(None));
    }
    let world = rig.world_mut();
    for n in 1..=rows {
        let walker = Walker(if n <= moving { Scalar::ONE } else { scalar(0.0) });
        world.spawn(key(n), REGION, (at(0.0), mesh(n), walker)).unwrap();
        if n <= presented {
            world.insert(key(n), Shown(1)).unwrap();
        }
    }
    (rig, sink)
}

/// A frame of the scene: a step, then the passes and the publication; what the pass wrote.
fn frame(rig: &mut Rig, sink: &NullSink) -> usize {
    rig.step();
    rig.passes();
    rig.publish();
    sink.0.swap(0, Ordering::Relaxed)
}

#[test]
fn a_pass_writes_the_moved_rows_and_the_snapshot_holds_the_presented_ones() {
    let (mut rig, sink) = scene(2_000, 1_000, 200, true, 2);
    assert_eq!(frame(&mut rig, &sink), 2_000, "the first pass is full");
    for _ in 0..3 {
        assert_eq!(frame(&mut rig, &sink), 1_000, "the rows that moved");
    }
    let frame = rig.reader.acquire().unwrap();
    assert_eq!(frame.newer().rows::<Shown>().len(), 200);
}

/// The times, at 50,000 rows moved among 100,000 with 10,000 presented rows, and with no
/// snapshot installed so the snapshot's part shows: the median and the least of the pass's
/// time over many frames, under one worker and four.
#[test]
#[ignore = "a measurement, run by hand: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "a measurement, run by hand: timings are reported")]
fn extract_at_fifty_thousand_moved_among_a_hundred_thousand() {
    const ROWS: u64 = 100_000;
    const MOVING: u64 = 50_000;
    const PRESENTED: u64 = 10_000;
    const FRAMES: usize = 40;
    for workers in [1, 4] {
        for snapshot in [true, false] {
            let (mut rig, sink) = scene(ROWS, MOVING, PRESENTED, snapshot, workers);
            frame(&mut rig, &sink);
            let mut times: Vec<Duration> = Vec::with_capacity(FRAMES);
            for _ in 0..FRAMES {
                rig.step();
                let start = Instant::now();
                rig.passes();
                times.push(start.elapsed());
                rig.publish();
                assert_eq!(sink.0.swap(0, Ordering::Relaxed), 50_000);
                if snapshot {
                    let frame = rig.reader.acquire().unwrap();
                    assert_eq!(frame.newer().rows::<Shown>().len(), 10_000);
                }
            }
            times.sort_unstable();
            println!(
                "extract, {MOVING} moved among {ROWS}, {} presented rows in the snapshot, \
                 {workers} worker(s): median {:?}, least {:?} over {FRAMES} passes",
                if snapshot { PRESENTED } else { 0 },
                times[FRAMES / 2],
                times[0],
            );
        }
    }
}
