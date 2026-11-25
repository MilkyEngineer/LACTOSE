//! The targets and the present thread (`README.md`, "The targets and the present thread"):
//! the protocol on a queue of its own and on a shared one, with a present side whose calls
//! the test holds.

use std::sync::Arc;
use std::time::Duration;

use lactose_app::{PresentReport, Region, Rendered};
use lactose_core::Tick;
use lactose_rhi_api::TimelinePoint;

use super::support::{
    Call, E1, Rig, SERIAL, TICK_HOOK, When, bounded, drew, later, lock, point, target,
};

const RELEASE: Duration = Duration::from_millis(50);

/// What holds of every run, from its log: no target drawn while a call that names it is
/// in flight, from the submit of the frame that queued it to its return; on a shared
/// queue, no `open` and no `submit` in that span either; and every draw's `ready` is the
/// read of the last call that named its target and reported one.
pub fn check_protocol(calls: &[Call], shared: bool) {
    for (end, call) in calls.iter().enumerate() {
        let Call::Returned { finished, .. } = call else { continue };
        let start = calls
            .iter()
            .position(|call| matches!(call, Call::Submit { point, .. } if *point == finished.point))
            .expect("a call follows the submit of its frame");
        for inner in &calls[start + 1..end] {
            match inner {
                Call::Draw(draw) => {
                    assert_ne!(
                        draw.target, finished.target,
                        "a target drawn while a call names it"
                    );
                }
                Call::Open | Call::Submit { .. } if shared => {
                    panic!("{inner:?} while a present call was in flight on a shared queue");
                }
                _ => {}
            }
        }
    }
    let mut reads: [Option<TimelinePoint>; lactose_app::TARGETS] = [None; lactose_app::TARGETS];
    for call in calls {
        match call {
            Call::Returned { finished, report: Some(PresentReport { read: Some(read), .. }) } => {
                reads[finished.target.index()] = Some(*read);
            }
            Call::Draw(draw) => {
                assert_eq!(draw.ready, reads[draw.target.index()], "`ready` is the last read");
            }
            _ => {}
        }
    }
}

#[test]
fn the_protocol_on_a_queue_of_its_own() {
    let mut rig = Rig::new(2, false);
    assert_eq!(drew(Some(rig.step(Some(E1)))), (target(0), true, false, false));
    rig.settle();
    rig.script.hold_presents(true);
    // The call of the first frame has returned: the second queues its own, which is held.
    assert_eq!(drew(Some(rig.step(Some(E1)))), (target(1), true, false, false));
    rig.script.await_presents(2);
    // While it is in flight, every frame draws into the other target and queues nothing.
    for _ in 0..2 {
        assert_eq!(drew(Some(rig.step(Some(E1)))), (target(0), false, false, false));
    }
    rig.script.hold_presents(false);
    rig.settle();
    assert_eq!(drew(Some(rig.step(Some(E1)))), (target(0), true, false, false));
    // A call that reports no read keeps nothing: the target's `ready` stays the read before.
    rig.script.report_answers([Ok(PresentReport { read: None, shown: false, waiting: None })]);
    rig.settle();
    assert_eq!(drew(Some(rig.step(Some(E1)))), (target(1), true, false, false));
    for _ in 0..3 {
        rig.settle();
        drew(Some(rig.step(Some(E1))));
    }
    let stats = rig.stats();
    assert_eq!((stats.drawn, stats.queued, stats.held), (9, 7, 0));
    rig.settle();
    let calls = rig.script.calls();
    // The eighth frame drew target 1 after the call that reported no read for it, with the
    // read before that call as its `ready`, which `check_protocol` holds it to.
    let draws: Vec<_> = calls
        .iter()
        .filter_map(|call| if let Call::Draw(draw) = call { Some(*draw) } else { None })
        .collect();
    assert_eq!(draws[7].target, target(1));
    assert!(draws[7].ready.is_some());
    check_protocol(&calls, false);
    rig.stop();
}

#[test]
fn the_protocol_on_a_shared_queue() {
    let mut rig = Rig::new(2, true);
    rig.script.hold_presents(true);
    // The first frame queues a call, held, and leaves no region open.
    rig.script.submit_answers([Ok(Region::Wait(point(90)))]);
    rig.frame(When::Ahead, 1, Some(E1));
    drew(rig.flush());
    rig.script.await_presents(1);
    let mark = rig.script.calls().len();
    // A frame that must ask `open` waits for the call until its limit: past it, busy, with
    // no `open` called.
    let busy = rig.frame(When::Passed, 1, Some(E1));
    assert_eq!(busy.deferred, Some(Rendered::Busy));
    assert!(busy.joined.is_none());
    let since = rig.script.calls().split_off(mark);
    assert!(since.iter().all(|call| !matches!(call, Call::Open | Call::Begin(_))));
    // Before it: the call returns, and the frame asks and draws.
    let script = Arc::clone(&rig.script);
    let release = later(RELEASE, move || script.hold_presents(false));
    let asked = rig.frame(When::Ahead, 1, Some(E1));
    release.join().expect("the release");
    assert_eq!(asked.deferred, None);
    assert!(rig.script.calls().contains(&Call::Open));
    drew(rig.flush());
    assert_eq!(rig.stats().busy, 1);

    // Held: a frame whose job returns while the call is in flight is submitted once the call
    // returns, and queues none. The release comes later on each try until a frame is held;
    // every frame before it was neither held nor shed, and queued.
    let mut delay = Duration::from_millis(20);
    let held = loop {
        rig.settle();
        rig.script.hold_presents(true);
        let queuing = rig.step(Some(E1));
        assert!(drew(Some(queuing)).1, "a frame neither held nor shed is queued");
        let script = Arc::clone(&rig.script);
        let release = later(delay, move || script.hold_presents(false));
        let next = rig.step(Some(E1));
        release.join().expect("the release");
        let (target_of, queued, held, shed) = drew(Some(next));
        assert!(!shed);
        if held {
            assert!(!queued, "a held frame is dropped");
            break target_of;
        }
        assert!(queued);
        assert!(delay < Duration::from_secs(5), "no frame was held");
        delay *= 2;
    };
    assert_eq!(rig.stats().held, 1);
    // The held frame's target is drawn again by the next frame.
    rig.settle();
    assert_eq!(drew(Some(rig.step(Some(E1)))).0, held);

    // `stop` with a render task held at its hold: it joins the task once the call returns.
    rig.settle();
    rig.script.hold_presents(true);
    drew(Some(rig.step(Some(E1))));
    rig.frame(When::Ahead, 1, Some(E1));
    let script = Arc::clone(&rig.script);
    let release = later(RELEASE, move || script.hold_presents(false));
    let calls_before = rig.script.calls().len();
    let script = Arc::clone(&rig.script);
    rig.stop();
    release.join().expect("the release");
    let calls = script.calls();
    assert_eq!(calls.last(), Some(&Call::Close), "close is the last call");
    assert!(calls.len() > calls_before);
    check_protocol(&calls, true);
}

/// A render task suspended at its hold occupies no worker: with two frame workers, the
/// ticks of later frames, each with the physics step's `block_in_place` at
/// `RECORD_RESERVATION`, run beside a job whose units are still running.
#[test]
fn a_held_render_task_leaves_the_workers_to_the_ticks_and_a_job() {
    let _serial = lock(&SERIAL);
    let mut rig = Rig::new(2, true);
    *lock(&TICK_HOOK) = Some(Arc::clone(&rig.script));
    rig.script.hold_presents(true);
    rig.frame(When::Ahead, 1, Some(E1));
    drew(rig.flush());
    rig.frame(When::Ahead, 1, Some(E1));
    rig.script.await_draws(2);
    let from = rig.shown().server().tick();
    // A job of three units, each waiting for a tick after `from` to start.
    let script = Arc::clone(&rig.script);
    let job = rig.executor.run_job(move |ctx| {
        let units = std::sync::atomic::AtomicU32::new(0);
        ctx.scope(|scope| {
            scope.fork_range(0..3, |range, _| {
                for _ in range {
                    script.until("a tick beside the job", |plan| {
                        plan.log
                            .iter()
                            .any(|call| matches!(call, Call::Tick(tick) if tick.0 > from.0))
                    });
                    units.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
        });
        units.into_inner()
    });
    for _ in 0..3 {
        let busy = rig.frame(When::Passed, 1, Some(E1));
        assert_eq!(busy.deferred, Some(Rendered::Busy));
    }
    assert_eq!(rig.shown().server().tick(), Tick(from.0 + 3));
    assert_eq!(bounded(&rig.executor, job, "the job beside the ticks").expect("the job"), 3);
    rig.script.hold_presents(false);
    drew(rig.flush());
    *lock(&TICK_HOOK) = None;
    rig.stop();
}
