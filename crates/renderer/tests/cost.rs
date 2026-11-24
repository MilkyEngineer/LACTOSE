//! Steering figures (`README.md`, "Proof", "It also owes"), printed and never asserted: the
//! ring's write cost for 50,000 rows, and `prepare`'s median and maximum at 50,000
//! incremental rows among 100,000 held, at a full pass of 100,000 held keys, and at a load
//! that replaces 100,000 held keys with 100,000 new ones. On null, whose buffers are host
//! memory, so the write figure is the writer's own cost and a device's upload memory may
//! differ; `prepare` reads nothing of the device but the flushes. The tests assert only the
//! counts that show each frame did what it measures.
//!
//! By hand, on a quiet machine with `uptime` before and after: `cargo test -p
//! lactose-renderer --release --test integration -- cost:: --ignored --nocapture
//! --test-threads 1`.

use std::time::{Duration, Instant};

use lactose_core::Tick;
use lactose_ecs::WorldKind;
use lactose_present_api::{ExtractPass, InstanceRow, InstanceSink};
use lactose_renderer::{FrameStats, Renderer, RendererDesc};

use crate::common::{Ender, null, ok, open_waiting, row};

const HELD: u32 = 100_000;
const INCREMENTAL: u32 = 50_000;
/// The rows of one `write`: what one job of extract hands the sink at a time.
const BATCH: usize = 256;
const FRAMES: usize = 21;

fn renderer() -> Renderer {
    let desc = RendererDesc { instances: HELD, rows: 2 * HELD + 8, regions: 3, shaders: None };
    ok(Renderer::new(null(), &desc))
}

fn rows(first: u64, count: u32, x: f32) -> Vec<InstanceRow> {
    (first..first + u64::from(count)).map(|n| row(n, x)).collect()
}

fn pass(renderer: &Renderer, tick: u32, full: bool, rows: &[InstanceRow]) -> Duration {
    let mut sink = renderer.sink();
    sink.begin(ExtractPass { kind: WorldKind::Server, tick: Tick(tick), full });
    let start = Instant::now();
    for batch in rows.chunks(BATCH) {
        sink.write(batch);
    }
    let took = start.elapsed();
    sink.end();
    took
}

fn timed_prepare(renderer: &mut Renderer) -> (FrameStats, Duration) {
    let start = Instant::now();
    let stats = ok(renderer.prepare());
    (stats, start.elapsed())
}

/// The median and the maximum, in milliseconds.
fn report(name: &str, mut times: Vec<Duration>) {
    times.sort_unstable();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let median = ms(times[times.len() / 2]);
    let max = ms(*times.last().expect("a frame"));
    println!(
        "steering: {name}: median {median:.3} ms, max {max:.3} ms over {} frames",
        times.len()
    );
}

/// Holds `HELD` keys, the frame ended and the next region open.
fn held(renderer: &mut Renderer, ender: &mut Ender) {
    pass(renderer, 1, true, &rows(1, HELD, 0.0));
    assert_eq!(ok(renderer.prepare()).live, HELD);
    ender.end(renderer);
    open_waiting(renderer);
}

#[test]
#[ignore = "a measured steering figure: run by hand"]
fn the_ring_write_cost_of_50k_rows() {
    let mut renderer = renderer();
    let mut ender = Ender::new(&**renderer.device());
    let written = rows(1, INCREMENTAL, 1.0);
    let mut times = Vec::new();
    for frame in 0..FRAMES as u32 {
        times.push(pass(&renderer, frame + 1, false, &written));
        let stats = ok(renderer.prepare());
        assert_eq!((stats.pass_rows, stats.dropped), (INCREMENTAL, 0));
        ender.end(&mut renderer);
        open_waiting(&mut renderer);
    }
    report("the ring's write of 50,000 rows", times);
}

#[test]
#[ignore = "a measured steering figure: run by hand"]
fn prepare_at_50k_incremental_rows_among_100k_held() {
    let mut renderer = renderer();
    let mut ender = Ender::new(&**renderer.device());
    held(&mut renderer, &mut ender);
    let mut times = Vec::new();
    for frame in 0..FRAMES as u32 {
        // Alternate halves, so every frame moves rows another frame did not.
        let first = 1 + u64::from(frame % 2) * u64::from(INCREMENTAL);
        pass(&renderer, frame + 2, false, &rows(first, INCREMENTAL, frame as f32));
        let (stats, took) = timed_prepare(&mut renderer);
        assert_eq!((stats.entries, stats.live, stats.allocated), (INCREMENTAL, HELD, 0));
        times.push(took);
        ender.end(&mut renderer);
        open_waiting(&mut renderer);
    }
    report("prepare, 50,000 incremental rows among 100,000 held", times);
}

#[test]
#[ignore = "a measured steering figure: run by hand"]
fn prepare_at_a_full_pass_of_100k_held_keys() {
    let mut renderer = renderer();
    let mut ender = Ender::new(&**renderer.device());
    held(&mut renderer, &mut ender);
    let mut times = Vec::new();
    for frame in 0..FRAMES as u32 {
        pass(&renderer, frame + 2, true, &rows(1, HELD, frame as f32));
        let (stats, took) = timed_prepare(&mut renderer);
        assert_eq!((stats.entries, stats.live, stats.freed), (HELD, HELD, 0));
        times.push(took);
        ender.end(&mut renderer);
        open_waiting(&mut renderer);
    }
    report("prepare, a full pass of 100,000 held keys", times);
}

#[test]
#[ignore = "a measured steering figure: run by hand"]
fn prepare_at_a_load_that_replaces_100k_held_keys() {
    let mut renderer = renderer();
    let mut ender = Ender::new(&**renderer.device());
    held(&mut renderer, &mut ender);
    let mut times = Vec::new();
    for frame in 0..FRAMES as u32 {
        // Each frame's keys are new: the previous frame's are all freed.
        let first = 1 + u64::from(frame + 1) * u64::from(HELD);
        pass(&renderer, frame + 2, true, &rows(first, HELD, 0.0));
        let (stats, took) = timed_prepare(&mut renderer);
        assert_eq!(
            (stats.freed, stats.allocated, stats.live, stats.unplaced),
            (HELD, HELD, HELD, 0)
        );
        times.push(took);
        ender.end(&mut renderer);
        open_waiting(&mut renderer);
    }
    report("prepare, a load that replaces 100,000 held keys", times);
}
