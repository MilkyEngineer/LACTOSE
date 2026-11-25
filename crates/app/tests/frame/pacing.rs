//! A paced frame's wait for the frame before (step 2), the bound on deferring for it, a
//! frame that looks once, and the frame's time (`README.md`, "The shown frame").

use std::time::Duration;

use lactose_app::{CATCH_UP_CAP, Rendered, ShownOutput, TICK_PERIOD};
use lactose_sim::TimeScale;

use super::support::{Call, E1, Rig, Setup, When, drew, later};

/// The release of a held side, late enough that the frame is usually waiting by then. No
/// assertion depends on it: a frame whose limit is an hour off joins whenever it comes.
const RELEASE: Duration = Duration::from_millis(50);

fn ran_passes(calls: &[Call]) -> bool {
    calls.iter().any(|call| matches!(call, Call::Begin(_)))
}

fn is_busy(output: &ShownOutput) -> bool {
    output.joined.is_none() && output.deferred == Some(Rendered::Busy)
}

/// The frame's presentations since the last take: their fractions.
fn fractions(rig: &Rig) -> Vec<f32> {
    rig.script
        .take_calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Present { time, .. } => Some(time.fraction),
            _ => None,
        })
        .collect()
}

#[test]
fn a_render_task_that_resolves_before_the_limit_is_joined_in_that_frame() {
    let mut rig = Rig::new(2, false);
    rig.script.hold_draws(true);
    let first = rig.frame(When::Ahead, 1, Some(E1));
    assert!(first.joined.is_none() && first.deferred.is_none());
    rig.script.await_draws(1);
    rig.script.take_calls();
    let script = std::sync::Arc::clone(&rig.script);
    let release = later(RELEASE, move || script.hold_draws(false));
    // The limit is an hour off: the frame waits for the task, takes it back and runs its
    // passes.
    let second = rig.frame(When::Ahead, 1, Some(E1));
    release.join().expect("the release");
    assert_eq!(second.world.ticks, 1);
    drew(second.joined);
    assert_eq!(second.deferred, None);
    let calls = rig.script.calls();
    assert!(ran_passes(&calls), "the frame that joined ran its passes");
    // The fraction of the frame that waited is the one its ticks left: half a period
    // owed, whatever the wait took.
    rig.flush();
    for fraction in fractions(&rig) {
        assert!((fraction - 0.5).abs() < 1e-3, "{fraction}");
    }
    rig.stop();
}

#[test]
fn a_render_task_in_flight_at_the_limit_leaves_the_frame_busy() {
    let mut rig = Rig::new(2, false);
    rig.script.hold_draws(true);
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(1);
    let tick = rig.shown().server().tick();
    rig.script.take_calls();

    // The limit has passed: the frame looks once, finds the task in flight, and is busy:
    // its ticks run, and nothing else.
    let busy = rig.frame(When::Passed, 2, Some(E1));
    assert!(is_busy(&busy));
    assert_eq!(busy.world.ticks, 2);
    assert_eq!(rig.shown().server().tick().0, tick.0 + 2);
    assert!(busy.world.gathered.is_empty(), "no pass, so no gather");
    let calls = rig.script.take_calls();
    assert!(calls.is_empty(), "no pass, no publication, no call of the render side: {calls:?}");

    // A frame that is behind: its limit is its `now` less the margin, which has passed.
    let behind = rig.frame(When::Now, CATCH_UP_CAP + 3, Some(E1));
    assert!(behind.world.behind);
    assert_eq!(behind.world.ticks, CATCH_UP_CAP);
    assert!(is_busy(&behind));
    assert!(rig.script.take_calls().is_empty());
    assert_eq!(rig.stats().busy, 2);

    // Released, the task is taken back by a later frame, which runs its passes.
    rig.script.hold_draws(false);
    let joined = rig.frame(When::Ahead, 1, Some(E1));
    drew(joined.joined);
    assert_eq!(joined.deferred, None);
    assert!(ran_passes(&rig.script.take_calls()));
    rig.flush();
    rig.stop();
}

#[test]
fn an_executor_of_one_frame_worker_looks_in_every_frame() {
    // A render task suspended at its hold, behind a present call the test holds, on a
    // shared queue: it occupies no worker, so the one worker still runs the ticks.
    let mut rig = Rig::new(1, true);
    rig.script.hold_presents(true);
    rig.frame(When::Ahead, 1, Some(E1));
    // The first task queued its call and resolved; the call is held.
    drew(rig.flush());
    rig.script.await_presents(1);
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(2);
    // The limit is an hour off, and the frame looks once all the same.
    for when in [When::Ahead, When::Passed] {
        let busy = rig.frame(when, 1, Some(E1));
        assert!(is_busy(&busy), "{when:?}");
        assert_eq!(busy.world.ticks, 1);
    }
    rig.script.hold_presents(false);
    drew(rig.flush());
    rig.stop();
}

#[test]
fn past_the_bound_a_frame_waits_for_the_frame_before_with_no_limit() {
    let setup = Setup { log_retention: Some(CATCH_UP_CAP + 2), ..Setup::new(2, false) };
    let mut rig = Rig::with(setup);
    rig.script.hold_draws(true);
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(1);
    // Two ticks since the passes is the bound: busy.
    for _ in 0..2 {
        assert!(is_busy(&rig.frame(When::Passed, 1, Some(E1))));
    }
    rig.script.take_calls();
    // The third is past it: the frame waits for the task however late, and its passes run.
    let script = std::sync::Arc::clone(&rig.script);
    let release = later(RELEASE, move || script.hold_draws(false));
    let waited = rig.frame(When::Passed, 1, Some(E1));
    release.join().expect("the release");
    drew(waited.joined);
    assert_eq!(waited.deferred, None);
    assert!(ran_passes(&rig.script.take_calls()));
    assert_eq!(rig.stats().busy, 2);
    rig.flush();
    rig.stop();
}

#[test]
fn the_fraction_is_the_share_of_a_period_past_the_newest_tick() {
    let mut rig = Rig::new(2, false);
    rig.frame(When::Ahead, 1, Some(E1));
    rig.flush();
    rig.script.take_calls();
    let fraction = |rig: &mut Rig, when: When, ticks: u32, owed: Duration| {
        rig.try_frame_owing(when, ticks, owed, Some(E1)).expect("the frame");
        rig.flush();
        let fractions = fractions(rig);
        assert_eq!(fractions.len(), 1);
        fractions[0]
    };
    // At the newest tick's deadline: zero.
    assert_eq!(fraction(&mut rig, When::Ahead, 1, Duration::ZERO), 0.0);
    // Half a period on.
    let half = fraction(&mut rig, When::Ahead, 1, TICK_PERIOD / 2);
    assert!((half - 0.5).abs() < 1e-3, "{half}");
    // Behind: the next tick is due now, a period on.
    assert_eq!(fraction(&mut rig, When::Now, CATCH_UP_CAP + 2, Duration::ZERO), 1.0);
    // Paused: one.
    rig.shown().server_mut().set_time_scale(TimeScale::PAUSED);
    assert_eq!(fraction(&mut rig, When::Ahead, 0, TICK_PERIOD / 4), 1.0);
    assert!(rig.shown().server().time_scale().is_paused());
    assert_eq!(fraction(&mut rig, When::Ahead, 3, TICK_PERIOD / 4), 1.0);
    rig.stop();
}
