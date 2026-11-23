//! The longest wait of a main-lane call while the window is dragged or resized
//! (`docs/verification.md`, the main lane's latency under a modal loop).
//!
//! ```text
//! cargo run -p lactose-platform --example main_lane_gap -- [SECONDS]
//! ```
//!
//! It opens a window and queues a main-lane call every 5 ms from another thread, as a
//! client's tasks do. Each call sends when it was queued and when it ran on a channel, so the
//! main thread waits on nothing of the example's. Once a second it prints, over the calls
//! that ran in that second: the longest wait of a call from its queueing to its run; the
//! longest gap between two runs, counting the gap from the last run of the second before;
//! the longest gap between two queueings, which is the queueing thread's own cadence on a
//! loaded machine; and the age of the oldest call still waiting to run, or "none". A second
//! in which the main thread was held shows the hold in that last figure, with "0 ran" when no
//! call ran in it, and the second the hold ends in shows it as a wait and a gap. At the end,
//! after `SECONDS` (30 by default) or when the window's close button is pressed, it prints
//! the same over the whole run.
//!
//! Drag the window by its title bar, or resize it by a corner, and hold it still with the
//! button down for a few seconds: the figure is the longest gap between runs in the seconds
//! that cover the hold, beside the gap between queueings, and the whole run's.

#![forbid(unsafe_code)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use lactose_core::{Result, ThreadClass};
use lactose_platform::{MainThread, PixelSize, WindowDesc, event_pump};
use lactose_task::{Executor, Platform, StdPlatform, ThreadSpec};

/// How often a call is queued.
const EVERY: Duration = Duration::from_millis(5);

/// The longest gap between two consecutive instants of `times`, which are in order.
fn longest_gap(times: impl Iterator<Item = Instant>) -> Duration {
    let mut gap = Duration::ZERO;
    let mut last = None;
    for time in times {
        if let Some(last) = last {
            gap = gap.max(time.saturating_duration_since(last));
        }
        last = Some(time);
    }
    gap
}

/// The longest wait of a call, each given as when it was queued and when it ran.
fn longest_wait(calls: &[(Instant, Instant)]) -> Duration {
    calls
        .iter()
        .map(|(queued, ran)| ran.saturating_duration_since(*queued))
        .max()
        .unwrap_or_default()
}

/// Prints one second's figures. `ran` and `queued` hold every call so far, in order, and
/// the first `printed` and `queued_printed` of them were in the seconds before.
fn print_second(
    second: u64,
    queued: &[Instant],
    ran: &[(Instant, Instant)],
    printed: usize,
    queued_printed: usize,
) {
    let now = Instant::now();
    let oldest = queued.get(ran.len()).map(|queued| now.saturating_duration_since(*queued));
    let oldest = oldest.map_or_else(|| "none".to_owned(), |age| format!("{age:.1?}"));
    let this = &ran[printed..];
    if this.is_empty() {
        println!("second {second:>3}:    0 ran, oldest waiting {oldest}");
        return;
    }
    let wait = longest_wait(this);
    let gap = longest_gap(ran[printed.saturating_sub(1)..].iter().map(|call| call.1));
    let cadence = longest_gap(queued[queued_printed.saturating_sub(1)..].iter().copied());
    println!(
        "second {second:>3}: {:>4} ran, longest wait {wait:>9.1?}, longest gap {gap:>9.1?} \
         (queued every {cadence:>9.1?} at most), oldest waiting {oldest}",
        this.len()
    );
}

fn main() -> Result<()> {
    let seconds = std::env::args().nth(1).and_then(|arg| arg.parse().ok()).unwrap_or(30);
    let (main, pump) = event_pump(WindowDesc {
        title: "main lane gap: drag or resize me".to_owned(),
        size: PixelSize { width: 800, height: 600 },
    })?;
    let (sent, received) = mpsc::channel::<(Instant, Instant)>();
    let (summary, finished) = mpsc::channel();
    let driver = {
        let main = main.clone();
        StdPlatform.spawn_thread(
            ThreadSpec {
                name: "driver".to_owned(),
                class: ThreadClass::FrameCritical,
                index_in_class: 0,
            },
            Box::new(move || {
                // The call runs once the window is open, and is the one the driver waits for.
                let executor = Executor::builder().build().expect("an executor");
                let Ok(reports) = executor.block_on(main.run(|main| main.window().reports()))
                else {
                    return;
                };
                executor.shutdown();
                // When each call was queued, and the calls that have run, both in order: the
                // driver is the lane's one sender, so its calls run in the order queued.
                let (mut queued, mut ran) = (Vec::new(), Vec::new());
                let (mut printed, mut queued_printed) = (0_usize, 0_usize);
                let start = Instant::now();
                let mut second = 1;
                while start.elapsed() < Duration::from_secs(seconds) && !reports.close_requested() {
                    let at = Instant::now();
                    queued.push(at);
                    let sent = sent.clone();
                    drop(main.run(move |_| {
                        let _ = sent.send((at, Instant::now()));
                    }));
                    std::thread::sleep(EVERY);
                    if start.elapsed() >= Duration::from_secs(second) {
                        ran.extend(received.try_iter());
                        print_second(second, &queued, &ran, printed, queued_printed);
                        (printed, queued_printed) = (ran.len(), queued.len());
                        second += 1;
                    }
                }
                drop(main.run(MainThread::exit));
                let _ = summary.send((queued, ran, received));
            }),
        )?
    };
    pump.run()?;
    driver.join();
    let Ok((queued, mut ran, received)) = finished.try_recv() else {
        return Ok(());
    };
    ran.extend(received.try_iter());
    println!(
        "{} calls ran of {} queued: longest wait {:.1?}, longest gap between runs {:.1?}, \
         longest gap between queueings {:.1?}",
        ran.len(),
        queued.len(),
        longest_wait(&ran),
        longest_gap(ran.iter().map(|call| call.1)),
        longest_gap(queued.iter().copied()),
    );
    Ok(())
}
