//! How a shown run ends (`README.md`, "Errors", "Shutdown"): the policy's two rows for the
//! render side, `stop` with a frame and a call in flight, after a shutdown and after a lost
//! device, and a paced frame met by a shutdown in its wait.

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use lactose_app::{ErrorPolicy, Response, ShownKept, Source, TICK_PERIOD};
use lactose_core::{Error, ErrorKind};
use lactose_sim::TickInput;
use lactose_task::WorkerClass;

use super::support::{Call, E1, Rig, When, drew, later, lock};

const RELEASE: Duration = Duration::from_millis(50);

#[test]
fn the_policy_stops_for_a_lost_device_and_logs_every_other_render_error() {
    let policy = ErrorPolicy::MILESTONE_2;
    let lost = Error::new(ErrorKind::DeviceLost, "lost");
    assert_eq!(policy.respond(Source::Render, &lost), Response::Stop);
    for kind in [ErrorKind::Platform, ErrorKind::ResourceExhausted, ErrorKind::Unsupported] {
        assert_eq!(policy.respond(Source::Render, &Error::new(kind, "other")), Response::Log);
    }
    assert_eq!(ErrorPolicy::MILESTONE_1.respond(Source::Render, &lost), Response::Stop);
}

/// A lost device from each method that can report one ends the run with the world and the
/// render side kept, and `stop` then ends them in order.
#[test]
fn a_lost_device_keeps_the_world_and_stop_ends_it() {
    for (open, draw) in [(true, false), (false, true)] {
        let mut rig = Rig::new(2, false);
        let lost = || Error::new(ErrorKind::DeviceLost, "the scripted device was lost");
        if open {
            rig.script.open_answers(&[super::support::Answer::Fail(ErrorKind::DeviceLost)]);
        } else {
            drew(Some(rig.step(Some(E1))));
        }
        if draw {
            rig.script.draw_answers([Err(lost())]);
        }
        let error = rig.try_step(Some(E1)).expect_err("a lost device stops the run");
        assert_eq!(error.error.kind(), ErrorKind::DeviceLost);
        assert!(matches!(error.kept, ShownKept::Nothing), "the rig took the world back");
        let script = Arc::clone(&rig.script);
        rig.stop();
        assert_eq!(script.calls().last(), Some(&Call::Close));
    }
    // A present call that reports a lost device, taken at the next decision.
    let mut rig = Rig::new(2, false);
    rig.script.report_answers([Err(Error::new(ErrorKind::DeviceLost, "lost at present"))]);
    drew(Some(rig.step(Some(E1))));
    rig.settle();
    let error = rig.try_step(Some(E1)).expect_err("a lost device stops the run");
    assert_eq!(error.error.kind(), ErrorKind::DeviceLost);
    rig.stop();
}

#[test]
fn stop_joins_the_frame_and_the_call_in_flight_and_closes_last() {
    for shared in [false, true] {
        let mut rig = Rig::new(2, shared);
        rig.script.hold_presents(true);
        drew(Some(rig.step(Some(E1))));
        rig.frame(When::Ahead, 1, Some(E1));
        rig.script.await_draws(2);
        let script = Arc::clone(&rig.script);
        let release = later(RELEASE, move || script.hold_presents(false));
        let script = Arc::clone(&rig.script);
        rig.stop();
        release.join().expect("the release");
        let calls = script.calls();
        assert_eq!(calls.last(), Some(&Call::Close), "close is the last call");
        let returned = calls.iter().filter(|call| matches!(call, Call::Returned { .. })).count();
        let called = calls.iter().filter(|call| matches!(call, Call::Called(_))).count();
        assert_eq!(returned, called, "every call in flight was awaited");
    }
}

#[test]
fn stop_with_no_frame_in_flight_after_a_shutdown_closes() {
    let mut rig = Rig::new(2, false);
    drew(Some(rig.step(Some(E1))));
    rig.executor.clone().shutdown();
    let script = Arc::clone(&rig.script);
    rig.stop();
    assert_eq!(script.calls().last(), Some(&Call::Close));
}

#[test]
fn stop_with_a_frame_in_flight_after_a_shutdown_is_cancelled() {
    let mut rig = Rig::new(2, true);
    rig.script.hold_presents(true);
    drew(Some(rig.step(Some(E1))));
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(2);
    let executor = rig.executor.clone();
    let shown = rig.take();
    executor.clone().shutdown();
    let stopped = super::support::bounded(&executor, shown.stop(), "the stop");
    let error = stopped.err().expect("the join is cancelled");
    assert_eq!(error.kind(), ErrorKind::Cancelled);
    rig.script.hold_presents(false);
}

/// A shutdown while a paced frame driven by `block_on` waits: `Cancelled`, with nothing
/// kept, whether the render task's join reports it (the task suspended, at its hold) or
/// the frame's `Sleep` does (the task inside a poll, in `submit`).
#[test]
fn a_frame_waiting_when_the_executor_shuts_down_is_cancelled_with_nothing_kept() {
    for in_poll in [false, true] {
        let mut rig = Rig::new(2, !in_poll);
        if in_poll {
            drew(Some(rig.step(Some(E1))));
            rig.script.hold_submits(true);
            rig.frame(When::Ahead, 1, Some(E1));
            rig.script.await_submits(2);
        } else {
            rig.script.hold_presents(true);
            drew(Some(rig.step(Some(E1))));
            rig.frame(When::Ahead, 1, Some(E1));
            rig.script.await_draws(2);
        }
        let executor = rig.executor.clone();
        let script = Arc::clone(&rig.script);
        let shutdown = later(RELEASE, move || {
            let releaser = {
                let script = Arc::clone(&script);
                // The worker inside `submit` is let go once the shutdown has begun, so that
                // the shutdown can join it.
                later(RELEASE * 4, move || script.hold_submits(false))
            };
            executor.shutdown();
            releaser.join().expect("the release");
        });
        let Err(error) = rig.try_frame(When::Ahead, 1, Some(E1)) else {
            panic!("the frame is cancelled");
        };
        assert_eq!(error.error.kind(), ErrorKind::Cancelled, "in a poll: {in_poll}");
        assert!(matches!(error.kept, ShownKept::Nothing));
        shutdown.join().expect("the shutdown");
        rig.script.hold_presents(false);
    }
}

/// A root task met in its wait by a shutdown is dropped unpolled, on the thread that shut
/// the executor down, and resolves nothing.
#[test]
fn a_root_frame_met_by_a_shutdown_is_dropped_where_the_shutdown_runs() {
    struct Probe(Arc<Mutex<Option<ThreadId>>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            *lock(&self.0) = Some(std::thread::current().id());
        }
    }
    let _serial = lock(&super::support::SERIAL);
    let mut rig = Rig::new(2, true);
    rig.script.hold_presents(true);
    drew(Some(rig.step(Some(E1))));
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(2);
    *lock(&super::support::TICK_HOOK) = Some(Arc::clone(&rig.script));
    let from = rig.shown().server().tick();
    let now = Instant::now() + Duration::from_secs(3600);
    rig.shown().server_mut().resync(now - TICK_PERIOD - TICK_PERIOD / 2);
    let shown = rig.take();
    let dropped = Arc::new(Mutex::new(None));
    let resolved = Arc::new(Mutex::new(false));
    let probe = Probe(Arc::clone(&dropped));
    let handle = rig.executor.clone();
    let done = Arc::clone(&resolved);
    let root = rig.executor.spawn(WorkerClass::FrameCritical, async move {
        let _probe = probe;
        let mut frames = |_| Some(TickInput::default());
        let outcome = shown.frame(&handle, now, &mut frames, Some(E1)).await;
        *lock(&done) = true;
        drop(outcome);
    });
    // The root has run its tick and goes on to wait for the task, held at its hold: the
    // pause after the tick is what lets it reach the wait, and is the one place the test
    // leans on time. A root still inside its poll at the shutdown would be dropped on its
    // worker, which the last assertion would report.
    rig.script.until("the root's tick", |plan| {
        plan.log.iter().any(|call| matches!(call, Call::Tick(tick) if tick.0 > from.0))
    });
    std::thread::sleep(RELEASE * 2);
    *lock(&super::support::TICK_HOOK) = None;
    let shutting = std::thread::current().id();
    rig.executor.clone().shutdown();
    assert!(!*lock(&resolved), "the frame resolved nothing");
    assert_eq!(*lock(&dropped), Some(shutting), "dropped on the thread that shut down");
    drop(root);
    rig.script.hold_presents(false);
}
