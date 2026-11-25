//! What extract's pass costs (`README.md`, "Proof for milestone 2", Cost): a server world of
//! instances in four archetypes and sixteen regions, half of which move every tick, a stated
//! count of them presented rows the snapshot holds, extracted to a null sink that only counts.
//! The presented rows' keys follow no chunk order: they are interleaved across the chunks of
//! both presented archetypes and of every region, as `extract-50k`'s scene lays them out
//! (D164), so the snapshot's fill is measured where an order is work to find. Asserted on what
//! does not depend on the machine (how many rows each pass writes, and the snapshot's rows);
//! the times are measured by an ignored test, run by hand in release, and reported, never
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

use common::{Rig, Shown, Walker, at, key, mesh, simulation};
use lactose_core::{StableKey, Tick};
use lactose_ecs::{LayoutConfig, RegionConfig, RegionId, RegionMode, WorldKind};
use lactose_math::Scalar;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink, extract_state};

/// The scene's regions: `common`'s two and fourteen more.
const REGIONS: u64 = 16;

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

/// The region of row `n`.
fn region(n: u64) -> RegionId {
    RegionId(1 + n % REGIONS)
}

/// A server world of `rows` instances in sixteen regions, row `n` in region `1 + n % 16`, the
/// even rows walking one unit a tick, and `presented` of them carrying `Shown`: those at 1 and
/// 2 modulo `2 * rows / presented`, so half the presented rows walk, and each presented
/// archetype's chunks interleave their keys with the other's and every region's. A null sink
/// and, if `snapshot`, a snapshot channel are installed.
fn scene(rows: u64, presented: u64, snapshot: bool, workers: u32) -> (Rig, NullSink) {
    let period = 2 * rows / presented;
    assert!(period >= 3 && period * presented == 2 * rows, "{presented} of {rows} rows");
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
    for id in 3..=REGIONS {
        let config = RegionConfig { id: RegionId(id), seed: 10 + id, mode: RegionMode::Simulated };
        world.create_region(config).expect("a region");
    }
    for n in 1..=rows {
        let walks = n % 2 == 0;
        let shown = matches!(n % period, 1 | 2);
        let spawned = match (walks, shown) {
            (false, false) => world.spawn(key(n), region(n), (at(0.0), mesh(n))),
            (false, true) => world.spawn(key(n), region(n), (at(0.0), mesh(n), Shown(1))),
            (true, false) => {
                world.spawn(key(n), region(n), (at(0.0), mesh(n), Walker(Scalar::ONE)))
            }
            (true, true) => {
                world.spawn(key(n), region(n), (at(0.0), mesh(n), Walker(Scalar::ONE), Shown(1)))
            }
        };
        spawned.unwrap();
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
    let (mut rig, sink) = scene(2_000, 200, true, 2);
    assert_eq!(frame(&mut rig, &sink), 2_000, "the first pass is full");
    for _ in 0..3 {
        assert_eq!(frame(&mut rig, &sink), 1_000, "the rows that moved");
    }
    let frame = rig.reader.acquire().unwrap();
    assert_eq!(frame.newer().rows::<Shown>().len(), 200);
    assert_eq!(frame.newer().rows::<Walker>().len(), 100, "half the presented rows walk");
    // No chunk order is key order: some chunk holds keys on both sides of another chunk's.
    let chunks: Vec<Vec<StableKey>> = rig
        .world()
        .chunks()
        .filter(|chunk| chunk.column(<Shown as lactose_reflect::Reflect>::KEY).is_some())
        .map(|chunk| chunk.keys().to_vec())
        .collect();
    assert!(chunks.len() >= 2 * usize::try_from(REGIONS).unwrap(), "{} chunks", chunks.len());
    let (first, second) = (&chunks[0], &chunks[1]);
    assert!(first.iter().any(|key| key < &second[0]) && first.iter().any(|key| key > &second[0]));
}

/// The arm of the ignored test that runs.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Arm {
    /// No snapshot installed: the ring's part alone.
    Ring,
    /// The snapshot installed, the presented rows staying where they are.
    Snapshot,
    /// The snapshot installed, and before every tick one presented row despawned and another
    /// spawned in its archetype and region: a pass whose kept places no longer hold, which
    /// sorts.
    Sorting,
}

/// The times, at 50,000 rows moved among 100,000, with 5,000 presented rows, the count
/// criterion 4 is judged at, and 10,000 printed beside it (D164): for each, the median and the
/// least of the pass's time over sixty frames with no snapshot installed, with one, and with
/// one whose order is broken at every tick, under one worker and four. The snapshot's part is
/// the median with it less the median without.
#[test]
#[ignore = "a measurement, run by hand: timings are reported, not asserted"]
#[expect(clippy::disallowed_types, reason = "a measurement, run by hand: timings are reported")]
fn extract_at_fifty_thousand_moved_among_a_hundred_thousand() {
    const ROWS: u64 = 100_000;
    const MOVING: usize = 50_000;
    // Decision 5.2 of the milestone 2 plan measures over 60 passes.
    const FRAMES: usize = 60;
    for presented in [5_000, 10_000] {
        let period = 2 * ROWS / presented;
        for workers in [1, 4] {
            let mut ring = Duration::ZERO;
            for arm in [Arm::Ring, Arm::Snapshot, Arm::Sorting] {
                let (mut rig, sink) = scene(ROWS, presented, arm != Arm::Ring, workers);
                frame(&mut rig, &sink);
                // The despawned row of the sorting arm: a presented row that does not walk.
                let mut gone = 1;
                let mut times: Vec<Duration> = Vec::with_capacity(FRAMES);
                for spawned in 1..=FRAMES {
                    if arm == Arm::Sorting {
                        let world = rig.world_mut();
                        let region = world.region_of(key(gone)).expect("a live row");
                        world.despawn(key(gone)).unwrap();
                        let fresh = key(ROWS + u64::try_from(spawned).unwrap());
                        world.spawn(fresh, region, (at(0.0), mesh(1), Shown(1))).unwrap();
                        gone += period;
                    }
                    rig.step();
                    let start = Instant::now();
                    rig.passes();
                    times.push(start.elapsed());
                    rig.publish();
                    let written = sink.0.swap(0, Ordering::Relaxed);
                    let forced = usize::from(arm == Arm::Sorting);
                    assert_eq!(written, MOVING + forced, "the moved rows and a spawned one");
                    if arm != Arm::Ring {
                        let frame = rig.reader.acquire().unwrap();
                        let held = frame.newer().rows::<Shown>().len();
                        assert_eq!(held, usize::try_from(presented).unwrap());
                    }
                }
                times.sort_unstable();
                let median = times[FRAMES / 2];
                if arm == Arm::Ring {
                    ring = median;
                }
                println!(
                    "extract, {MOVING} moved among {ROWS}, {presented} presented rows, {arm:?}, \
                     {workers} worker(s): median {median:?}, least {:?} over {FRAMES} passes; \
                     the snapshot's part {:?}",
                    times[0],
                    median.saturating_sub(ring),
                );
            }
        }
    }
}
