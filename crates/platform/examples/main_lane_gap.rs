//! The longest wait of a main-lane call while the window is dragged or resized
//! (`docs/verification.md`, the main lane's latency under a modal loop).
//!
//! ```text
//! cargo run -p lactose-platform --example main_lane_gap -- [SECONDS]
//! ```
//!
//! It opens a window and queues a main-lane call every 5 ms from another thread, as a
//! client's tasks do. Each second it prints the longest wait of a call from its queueing
//! to its run, the longest gap between two runs and the longest between two queueings, in
//! that second; at the end, after
//! `SECONDS` (30 by default) or when the window's close button is pressed, it prints the
//! same over the whole run. Drag the window by its title bar, or resize it by a corner,
//! and hold it still with the button down for a few seconds: the seconds in which that
//! happened show what the operating system's modal loop does to the main lane.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lactose_core::{Result, ThreadClass};
use lactose_platform::{MainThread, PixelSize, WindowDesc, event_pump};
use lactose_task::{Executor, Platform, StdPlatform, ThreadSpec};

/// How often a call is queued.
const EVERY: Duration = Duration::from_millis(5);

/// Among a run of calls, each given as when it was queued and when it ran: the longest wait
/// of a call, the longest gap between two runs, and the longest gap between two queueings.
/// A gap between runs is the lane's only where it is longer than the queueings' gap, which
/// is the driver's own cadence on a loaded machine.
fn longest(calls: &[(Instant, Instant)]) -> (Duration, Duration, Duration) {
    let gap = |at: fn(&(Instant, Instant)) -> Instant| {
        let gaps =
            calls.windows(2).map(|pair| at(&pair[1]).saturating_duration_since(at(&pair[0])));
        gaps.max().unwrap_or_default()
    };
    let wait = calls.iter().map(|(queued, ran)| ran.saturating_duration_since(*queued)).max();
    (wait.unwrap_or_default(), gap(|call| call.1), gap(|call| call.0))
}

fn main() -> Result<()> {
    let seconds = std::env::args().nth(1).and_then(|arg| arg.parse().ok()).unwrap_or(30);
    let (main, pump) = event_pump(WindowDesc {
        title: "main lane gap: drag or resize me".to_owned(),
        size: PixelSize { width: 800, height: 600 },
    })?;
    let runs = Arc::new(Mutex::new(Vec::new()));
    let driver = {
        let (main, runs) = (main.clone(), runs.clone());
        StdPlatform.spawn_thread(
            ThreadSpec { name: "driver".to_owned(), class: ThreadClass::FrameCritical, index_in_class: 0 },
            Box::new(move || {
                // The call runs once the window is open, and is the one the driver waits for.
                let executor = Executor::builder().build().expect("an executor");
                let Ok(reports) = executor.block_on(main.run(|main| main.window().reports())) else {
                    return;
                };
                executor.shutdown();
                let start = Instant::now();
                let mut second = 1;
                while start.elapsed() < Duration::from_secs(seconds) && !reports.close_requested() {
                    let queued = Instant::now();
                    let ran = runs.clone();
                    drop(main.run(move |_| {
                        ran.lock().expect("not poisoned").push((queued, Instant::now()));
                    }));
                    std::thread::sleep(EVERY);
                    if start.elapsed() >= Duration::from_secs(second) {
                        let since = start + Duration::from_secs(second - 1);
                        let calls = runs.lock().expect("not poisoned");
                        let first = calls.partition_point(|(queued, _)| *queued < since);
                        let (wait, gap, queued) = longest(&calls[first..]);
                        println!(
                            "second {second:>3}: longest wait {wait:>9.1?}, longest gap {gap:>9.1?} (queued every {queued:>9.1?} at most)"
                        );
                        second += 1;
                    }
                }
                drop(main.run(MainThread::exit));
            }),
        )?
    };
    pump.run()?;
    driver.join();
    let calls = runs.lock().expect("not poisoned");
    let (wait, gap, queued) = longest(&calls);
    println!(
        "{} calls: longest wait {wait:.1?}, longest gap between runs {gap:.1?}, longest gap between queueings {queued:.1?}",
        calls.len()
    );
    Ok(())
}
