//! The present side's count, `PresentReport::waiting`, over scripted display-time reports
//! and timeline values (`crates/app/README.md`, "The shedding rule", *The signal*).

use std::time::Instant;

use lactose_app_client::Waiting;
use lactose_rhi_api::{DisplayTime, PresentId};

fn shown(present: u64) -> DisplayTime {
    DisplayTime { present: PresentId(present), shown: Instant::now() }
}

/// Presents 1 to `n`, the read of present `k` signalling `k`.
fn made(n: u64) -> Waiting {
    let mut waiting = Waiting::new();
    for k in 1..=n {
        waiting.presented(PresentId(k), k);
    }
    waiting
}

#[test]
fn no_source_of_display_times_counts_nothing_and_keeps_nothing() {
    let mut waiting = made(5);
    assert_eq!(waiting.take(false, &[], 5), None);
    assert_eq!(waiting.kept(), 0);
    // Reports that arrive with no source are not read.
    let mut waiting = made(5);
    assert_eq!(waiting.take(false, &[shown(5)], 5), None);
}

#[test]
fn a_present_still_behind_the_gpu_is_not_counted() {
    let mut waiting = made(4);
    // Reads 1 and 2 reached, 3 and 4 not: two finished, none displayed.
    assert_eq!(waiting.take(true, &[], 2), Some(2));
    assert_eq!(waiting.take(true, &[], 4), Some(4));
    assert_eq!(waiting.kept(), 4);
}

#[test]
fn a_displayed_present_covers_itself_and_every_earlier_one() {
    let mut waiting = made(5);
    // Present 3 named displayed: 1 to 3 are gone, whether or not 1 and 2 were named.
    assert_eq!(waiting.take(true, &[shown(3)], 5), Some(2));
    assert_eq!(waiting.kept(), 2);
    // Reports since the last call only: none, and the two remain.
    assert_eq!(waiting.take(true, &[], 5), Some(2));
    // Several reports in one call, oldest first: the newest decides.
    assert_eq!(waiting.take(true, &[shown(4), shown(5)], 5), Some(0));
    assert_eq!(waiting.kept(), 0);
}

#[test]
fn the_count_rises_one_a_frame_while_the_display_takes_nothing() {
    let mut waiting = Waiting::new();
    let mut counts = Vec::new();
    for k in 1..=4_u64 {
        // A call begins: every earlier read reached, nothing displayed.
        counts.push(waiting.take(true, &[], k - 1));
        waiting.presented(PresentId(k), k);
    }
    assert_eq!(counts, [Some(0), Some(1), Some(2), Some(3)]);
    // The display takes present 4: nothing is left waiting.
    assert_eq!(waiting.take(true, &[shown(4)], 4), Some(0));
}

#[test]
fn a_report_of_a_present_never_made_here_drops_nothing_later() {
    let mut waiting = Waiting::new();
    waiting.presented(PresentId(7), 1);
    waiting.presented(PresentId(8), 2);
    // An id below every kept one changes nothing.
    assert_eq!(waiting.take(true, &[shown(6)], 2), Some(2));
    assert_eq!(waiting.kept(), 2);
}
