//! The shedding rule (`README.md`, "Pacing", *The shedding rule*), on each queue layout,
//! with a present side that reports the counts it is told.
//!
//! The frames are steps, each after the calls queued before it have returned, so that every
//! frame takes the report of the call before it: at its decision on a queue of its own, at
//! its hold on a shared one. A report is scripted for each call in the order the calls are
//! queued, so a frame that queues none leaves the next report to the next call. Where a gap
//! is tested, the pattern is the README's: the report before the gap reads the depth, the
//! `SHED_DEPTH` reports after it below it and the next two at it, and no frame is shed; an
//! order that passes over one report early sheds on the first of the last two, and one
//! that passes over one late sheds on the second.

use std::sync::Arc;
use std::time::Duration;

use lactose_app::{PresentReport, Region, Rendered, SHED_DEPTH};
use lactose_core::{Error, ErrorKind};

use super::support::{Answer, E1, Rig, When, drew, later, point};

/// A call's report: a count, no count, or an error.
#[derive(Clone, Copy, Debug)]
enum Rep {
    At(u32),
    None,
    Fail,
}

use Rep::{At, Fail};

fn report(rep: Rep) -> lactose_core::Result<PresentReport> {
    let read = Some(super::support::read_point(1_000));
    match rep {
        At(count) => Ok(PresentReport { read, shown: true, waiting: Some(count) }),
        Rep::None => Ok(PresentReport { read, shown: true, waiting: None }),
        Fail => Err(Error::new(ErrorKind::Platform, "the scripted present failed")),
    }
}

/// The pattern after a gap, which sheds nothing where the gap passes over exactly the
/// `SHED_DEPTH` reports after the one before it.
const AFTER_GAP: [Rep; 5] = [At(1), At(1), At(1), At(SHED_DEPTH), At(SHED_DEPTH)];

/// A rig and the calls it has queued, by their number from one.
struct Shedder {
    rig: Rig,
    calls: usize,
    /// For each frame shed, the number of the call whose report shed it.
    shed_by: Vec<usize>,
}

impl Shedder {
    fn new(shared: bool) -> Self {
        assert_eq!(SHED_DEPTH, 3, "the patterns below are written for a depth of three");
        Self { rig: Rig::new(2, shared), calls: 0, shed_by: Vec::new() }
    }

    fn note(&mut self, rendered: Option<Rendered>) {
        if let Some(Rendered::Drew { queued, shed, .. }) = rendered {
            if shed {
                self.shed_by.push(self.calls);
            }
            self.calls += usize::from(queued);
        }
    }

    /// One frame, after every call queued has returned.
    fn step(&mut self, window: Option<lactose_rhi_api::Extent>) -> Rendered {
        self.rig.settle();
        let rendered = self.rig.step(window);
        self.note(Some(rendered));
        rendered
    }

    /// Frames until a call has been queued for each of `reports`.
    fn run(&mut self, reports: &[Rep]) {
        let until = self.calls + reports.len();
        self.rig.script.report_answers(reports.iter().map(|rep| report(*rep)));
        while self.calls < until {
            self.step(Some(E1));
        }
    }

    /// One frame more, which takes the last report and queues a call reporting no count.
    fn tail(&mut self) {
        self.step(Some(E1));
    }

    /// A frame that queues a call reporting `rep`, then a frame dropped for that call.
    fn dropped(&mut self, rep: Rep, shared: bool) {
        let mut delay = Duration::from_millis(20);
        loop {
            self.rig.settle();
            self.rig.script.hold_presents(true);
            self.rig.script.report_answers([report(rep)]);
            let queuing = self.step(Some(E1));
            assert!(drew(Some(queuing)).1);
            if !shared {
                // No settle: the call is held, and this frame is dropped for it.
                let next = self.rig.step(Some(E1));
                self.note(Some(next));
                let (_, queued, held, _) = drew(Some(next));
                assert!(!queued && !held, "dropped for the call in flight");
                self.rig.script.hold_presents(false);
                return;
            }
            // On a shared queue the frame is dropped where its submit was held, which needs
            // the call still out when its job returns: the release comes later each try.
            let script = Arc::clone(&self.rig.script);
            let release = later(delay, move || script.hold_presents(false));
            let next = self.rig.step(Some(E1));
            release.join().expect("the release");
            self.note(Some(next));
            let (_, queued, held, shed) = drew(Some(next));
            assert!(!shed, "a held frame is not counted shed");
            if held {
                assert!(!queued);
                return;
            }
            assert!(delay < Duration::from_secs(5), "no frame was held");
            delay *= 2;
        }
    }
}

fn no_count_or_below_sheds_nothing(shared: bool) {
    let mut shedder = Shedder::new(shared);
    shedder.run(&[Rep::None; 4]);
    shedder.run(&[At(0), At(1), At(2), At(1), At(2), At(2)]);
    shedder.tail();
    assert!(shedder.shed_by.is_empty());
    assert_eq!(shedder.rig.stats().shed, 0);
    shedder.rig.stop();
}

#[test]
fn no_frame_is_shed_while_the_count_is_none_or_below_the_depth_on_either_layout() {
    no_count_or_below_sheds_nothing(false);
    no_count_or_below_sheds_nothing(true);
}

fn a_rise_sheds_one_frame(shared: bool) {
    let mut shedder = Shedder::new(shared);
    // A rise to the depth sheds the frame after its report, which draws into the target
    // the next frame draws into again; the count staying at the depth sheds no other.
    shedder.run(&[At(1), At(SHED_DEPTH)]);
    let shed = shedder.step(Some(E1));
    let (target, queued, held, was_shed) = drew(Some(shed));
    assert!(was_shed && !queued && !held, "{shed:?}");
    let next = drew(Some(shedder.step(Some(E1))));
    assert_eq!(next.0, target, "the shed frame's target is drawn again");
    // The next three reports are passed over: below and then at the depth among them shed
    // nothing, and a report with no count (the call the frame above queued, and the next)
    // and a call that failed are not among them.
    shedder.run(&[Rep::None, At(1), Fail, At(1), At(SHED_DEPTH), At(SHED_DEPTH)]);
    shedder.run(&[At(SHED_DEPTH); 2]);
    // A report below and a later one at the depth shed again.
    shedder.run(&[At(1), At(SHED_DEPTH)]);
    shedder.tail();
    assert_eq!(shedder.shed_by, vec![2, 13]);
    let stats = shedder.rig.stats();
    assert_eq!((stats.shed, stats.held), (2, 0));
    assert_eq!(stats.drawn - stats.queued, 2);
    shedder.rig.stop();
}

#[test]
fn a_rise_to_the_depth_sheds_one_frame_and_passes_over_the_reports_after_it() {
    a_rise_sheds_one_frame(false);
    a_rise_sheds_one_frame(true);
}

#[test]
fn the_first_report_after_a_failed_call_sheds_nothing() {
    for shared in [false, true] {
        let mut shedder = Shedder::new(shared);
        shedder.run(&[At(1), Fail, At(SHED_DEPTH)]);
        shedder.tail();
        assert!(shedder.shed_by.is_empty(), "{:?}", shedder.shed_by);
        shedder.rig.stop();
    }
}

fn after_a_dropped_frame(shared: bool) {
    let mut shedder = Shedder::new(shared);
    // On a queue of its own the report before the gap's reads below the depth, so that the
    // gap alone keeps the gap's from rising. On a shared one it reads the depth too: a try
    // whose frame was not held takes the gap's report as any other, which must not rise.
    shedder.run(if shared { &[Rep::None, At(SHED_DEPTH)] } else { &[At(1), At(1)] });
    // The report of the call a frame was dropped for reads the depth: no shed, and the
    // three after it are passed over.
    shedder.dropped(At(SHED_DEPTH), shared);
    shedder.run(&AFTER_GAP);
    shedder.tail();
    assert!(shedder.shed_by.is_empty(), "{:?}", shedder.shed_by);
    shedder.rig.stop();
}

#[test]
fn the_reports_after_a_dropped_frame_are_passed_over_on_a_queue_of_its_own() {
    after_a_dropped_frame(false);
}

#[test]
fn the_reports_after_a_held_frame_are_passed_over_on_a_shared_queue() {
    after_a_dropped_frame(true);
}

/// What a frame that draws nothing is.
#[derive(Clone, Copy, Debug)]
enum Nothing {
    Waited,
    Hidden,
    Torn,
}

impl Nothing {
    fn run(self, shedder: &mut Shedder) {
        let rendered = match self {
            Self::Hidden => shedder.step(None),
            Self::Waited | Self::Torn => {
                // The region goes at the frame before, and `open` answers for this one.
                let answer = match self {
                    Self::Waited => Answer::Wait,
                    _ => Answer::Fail(ErrorKind::Platform),
                };
                shedder.rig.script.open_answers(&[answer]);
                shedder.step(Some(E1))
            }
        };
        let expected = match self {
            Self::Waited => matches!(rendered, Rendered::Waited(_)),
            Self::Hidden => rendered == Rendered::Hidden,
            Self::Torn => rendered == Rendered::Torn,
        };
        assert!(expected, "{self:?}: {rendered:?}");
    }
}

/// A frame that drew nothing, once with no call out (where the passes over begin at the next
/// report) and once with one (where they follow that call's report, which sheds nothing).
fn after_nothing(nothing: Nothing, shared: bool) {
    // No call out: a waited or torn frame on a shared queue has taken the call at step 3
    // before it asked; on a queue of its own, and for a hidden frame, the first frame of
    // the run has none.
    let mut shedder = Shedder::new(shared);
    if matches!(nothing, Nothing::Hidden) || !shared {
        // The run's first frame, which asks for a region where it is not hidden.
        nothing.run(&mut shedder);
    } else {
        shedder.run(&[At(SHED_DEPTH)]);
        shedder.rig.script.submit_answers([Ok(Region::Wait(point(80)))]);
        shedder.run(&[At(SHED_DEPTH)]);
        nothing.run(&mut shedder);
    }
    shedder.run(&AFTER_GAP[..]);
    shedder.tail();
    assert!(shedder.shed_by.is_empty(), "{nothing:?}, none out: {:?}", shedder.shed_by);
    shedder.rig.stop();

    // A call out, where the layout leaves one: its report reads the depth after one below.
    let mut shedder = Shedder::new(shared);
    shedder.run(&[At(1), At(1)]);
    let out = match nothing {
        Nothing::Hidden => true,
        Nothing::Waited | Nothing::Torn => !shared,
    };
    if !out {
        shedder.rig.stop();
        return;
    }
    if !matches!(nothing, Nothing::Hidden) {
        shedder.rig.script.submit_answers([Ok(Region::Wait(point(81)))]);
    }
    shedder.run(&[At(SHED_DEPTH)]);
    nothing.run(&mut shedder);
    shedder.run(&AFTER_GAP);
    shedder.tail();
    assert!(shedder.shed_by.is_empty(), "{nothing:?}, one out: {:?}", shedder.shed_by);
    shedder.rig.stop();
}

#[test]
fn the_reports_after_a_frame_that_drew_nothing_are_passed_over() {
    for shared in [false, true] {
        for nothing in [Nothing::Waited, Nothing::Hidden, Nothing::Torn] {
            after_nothing(nothing, shared);
        }
    }
}

#[test]
fn a_frame_torn_at_its_draw_with_a_call_out_passes_over_the_reports_after_that_call() {
    for shared in [false, true] {
        let mut shedder = Shedder::new(shared);
        shedder.run(&[At(1), At(1), At(SHED_DEPTH)]);
        shedder.rig.script.draw_answers([Err(Error::new(ErrorKind::Platform, "scripted"))]);
        assert_eq!(shedder.step(Some(E1)), Rendered::Torn);
        shedder.run(&AFTER_GAP);
        shedder.tail();
        assert!(shedder.shed_by.is_empty(), "{:?}", shedder.shed_by);
        shedder.rig.stop();
    }
}

#[test]
fn a_run_of_hidden_frames_passes_over_the_reports_after_its_last_and_no_more() {
    for shared in [false, true] {
        let mut shedder = Shedder::new(shared);
        for _ in 0..3 {
            shedder.step(None);
        }
        // Three passed over, then one below and one at the depth: one shed, which a count
        // that added three for each hidden frame would not make.
        shedder.run(&[At(1), At(1), At(1), At(1), At(SHED_DEPTH)]);
        shedder.tail();
        assert_eq!(shedder.shed_by, vec![5]);
        shedder.rig.stop();
    }
}

#[test]
fn a_frame_busy_at_step_2_passes_over_from_the_report_of_the_late_task_s_call() {
    let mut shedder = Shedder::new(false);
    shedder.run(&[Rep::None, At(SHED_DEPTH)]);
    shedder.rig.settle();
    // The late frame takes the depth's report and queues the first of the pattern.
    shedder.rig.script.report_answers(AFTER_GAP[..1].iter().map(|rep| report(*rep)));
    shedder.rig.script.hold_draws(true);
    shedder.rig.frame(When::Ahead, 1, Some(E1));
    shedder.rig.script.await_draws(1);
    let busy = shedder.rig.frame(When::Passed, 1, Some(E1));
    assert_eq!(busy.deferred, Some(Rendered::Busy));
    shedder.rig.script.hold_draws(false);
    let late = shedder.rig.flush();
    shedder.note(late);
    assert!(drew(late).1, "the late task queued its call");
    shedder.run(&AFTER_GAP[1..]);
    shedder.tail();
    assert!(shedder.shed_by.is_empty(), "{:?}", shedder.shed_by);
    shedder.rig.stop();
}

#[test]
fn a_frame_busy_at_step_3_passes_over_the_reports_after_the_call_it_waited_behind() {
    let mut shedder = Shedder::new(true);
    shedder.run(&[At(1), At(1)]);
    shedder.rig.settle();
    // A call reading the depth, held, and no region after its frame.
    shedder.rig.script.hold_presents(true);
    shedder.rig.script.report_answers([report(At(SHED_DEPTH))]);
    shedder.rig.script.submit_answers([Ok(Region::Wait(point(82)))]);
    let rendered = shedder.step(Some(E1));
    assert!(drew(Some(rendered)).1);
    let busy = shedder.rig.frame(When::Passed, 1, Some(E1));
    assert_eq!(busy.deferred, Some(Rendered::Busy));
    shedder.rig.script.hold_presents(false);
    shedder.run(&AFTER_GAP);
    shedder.tail();
    assert!(shedder.shed_by.is_empty(), "{:?}", shedder.shed_by);
    shedder.rig.stop();
}

#[test]
fn a_rise_taken_by_step_3_before_a_frame_that_waited_sheds_nothing() {
    let mut shedder = Shedder::new(true);
    shedder.run(&[At(1)]);
    shedder.rig.script.submit_answers([Ok(Region::Wait(point(83)))]);
    shedder.run(&[At(SHED_DEPTH)]);
    // Step 3 takes the rise, and the frame then waits for a region.
    Nothing::Waited.run(&mut shedder);
    shedder.run(&[At(1); 4]);
    shedder.tail();
    assert!(shedder.shed_by.is_empty(), "{:?}", shedder.shed_by);
    shedder.rig.stop();
}
