//! The frame's three states and their order of calls, the four cases of a new sink, and a
//! starved frame (`README.md`, "The frame's three states", "Counts, torn frames and
//! resets"), by steps, so that every frame is joined before the next.

use std::sync::Arc;

use lactose_app::{
    CATCH_UP_CAP, FrameCounts, KindCounts, Region, Rendered, Start, WorldConfig, build_server,
};
use lactose_core::{Error, ErrorKind, Tick};

use super::support::{Answer, Call, E1, GAME, REGION, Rig, Setup, bounded, drew, target};
use crate::headless::InMemory;

/// The render side's calls, as short names a test lists: `begin+` is a full pass's begin.
fn shape(calls: &[Call]) -> Vec<String> {
    calls
        .iter()
        .filter(|call| call.is_render() || matches!(call, Call::Begin(_) | Call::End { .. }))
        .map(|call| match call {
            Call::Sink => "sink".to_owned(),
            Call::Open => "open".to_owned(),
            Call::Begin(pass) if pass.full => "begin+".to_owned(),
            Call::Begin(_) => "begin".to_owned(),
            Call::End { .. } => "end".to_owned(),
            Call::Present { .. } => "present".to_owned(),
            Call::Draw(draw) => format!("draw{}", draw.target.index()),
            Call::Submit { target, .. } => format!("submit{}", target.index()),
            other => panic!("not a render call: {other:?}"),
        })
        .collect()
}

/// One step after the present calls queued before it have returned, so that every
/// decision finds the call in flight returned: what it rendered and the calls it made.
fn step(rig: &mut Rig, window: Option<lactose_rhi_api::Extent>) -> (Rendered, Vec<String>) {
    rig.settle();
    let rendered = rig.step(window);
    let calls = rig.script.take_calls();
    // Every presentation reads the snapshot its own frame published.
    let tick = rig.shown().server().tick();
    for call in &calls {
        if let Call::Present { newer, .. } = call {
            assert_eq!(*newer, tick, "a frame presents the snapshot it published");
        }
    }
    (rendered, shape(&calls))
}

fn calls(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn each_state_in_order(shared: bool) {
    let mut rig = Rig::new(2, shared);
    assert_eq!(shape(&rig.script.take_calls()), calls(&["sink"]), "the build installs a sink");

    // Drawn, asking for a region first.
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(0), true, false, false));
    assert_eq!(made, calls(&["open", "begin+", "end", "present", "draw0", "submit0"]));
    // Drawn with a region open: no `open`, a pass that is not full, the other target.
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(1), true, false, false));
    assert_eq!(made, calls(&["begin", "end", "present", "draw1", "submit1"]));

    // Hidden: the sink detached, the passes run with none, the presentation alone.
    let (rendered, made) = step(&mut rig, None);
    assert_eq!(rendered, Rendered::Hidden);
    assert_eq!(made, calls(&["present"]));
    // Back from hiding: a new sink and a full pass.
    rig.script.submit_answers([Ok(Region::Wait(super::support::point(99)))]);
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(0), true, false, false));
    assert_eq!(made, calls(&["sink", "begin+", "end", "present", "draw0", "submit0"]));

    // Waits: the submit left no region, and `open` answers `Wait`: nothing else runs.
    rig.script.open_answers(&[Answer::Wait]);
    let (rendered, made) = step(&mut rig, Some(E1));
    assert!(matches!(rendered, Rendered::Waited(_)), "{rendered:?}");
    assert_eq!(made, calls(&["open"]));
    // The next frame asks again, and draws.
    rig.script.submit_answers([Ok(Region::Wait(super::support::point(98)))]);
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(1), true, false, false));
    assert_eq!(made, calls(&["open", "begin", "end", "present", "draw1", "submit1"]));

    // Torn at `open`: deferred, and a new sink owed.
    rig.script.open_answers(&[Answer::Fail(ErrorKind::ResourceExhausted)]);
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(rendered, Rendered::Torn);
    assert_eq!(made, calls(&["open"]));
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(0), true, false, false));
    assert_eq!(made, calls(&["open", "sink", "begin+", "end", "present", "draw0", "submit0"]));

    // Torn at `draw`: no submit, no present queued, the region asked for again and a new
    // sink at the next frame, which draws into the same target.
    rig.script.draw_answers([Err(Error::new(ErrorKind::ResourceExhausted, "scripted"))]);
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(rendered, Rendered::Torn);
    assert_eq!(made, calls(&["begin", "end", "present", "draw1"]));
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(1), true, false, false));
    assert_eq!(made, calls(&["open", "sink", "begin+", "end", "present", "draw1", "submit1"]));

    // Torn at `submit`: as at `draw`, after the submit's call.
    rig.script.submit_answers([Err(Error::new(ErrorKind::ResourceExhausted, "scripted"))]);
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(rendered, Rendered::Torn);
    assert_eq!(made, calls(&["begin", "end", "present", "draw0", "submit0"]));
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(drew(Some(rendered)), (target(0), true, false, false));
    assert_eq!(made, calls(&["open", "sink", "begin+", "end", "present", "draw0", "submit0"]));

    let stats = rig.stats();
    assert_eq!(stats.drawn, 7);
    assert_eq!(stats.queued, 7);
    assert_eq!(stats.hidden, 1);
    assert_eq!(stats.waited, 1);
    assert_eq!(stats.torn, 3);
    assert_eq!(stats.resets, 4);
    assert_eq!((stats.busy, stats.held, stats.shed), (0, 0, 0));
    rig.stop();
}

#[test]
fn each_state_calls_the_render_side_in_order_on_a_queue_of_its_own() {
    each_state_in_order(false);
}

#[test]
fn each_state_calls_the_render_side_in_order_on_a_shared_queue() {
    each_state_in_order(true);
}

fn server(counts: KindCounts) -> FrameCounts {
    FrameCounts { server: counts, ..FrameCounts::default() }
}

/// Cases 3 and 4 of a new sink, and no second reset after a full pass that left rows out
/// (cases 1 and 2 are in the order of calls above).
#[test]
fn counts_owe_a_new_sink_unless_a_full_pass_left_rows_out() {
    let mut rig = Rig::new(2, false);
    rig.script.take_calls();
    let dropped = server(KindCounts { lost: false, dropped: 1, unplaced: 0 });
    let unplaced = server(KindCounts { lost: false, dropped: 0, unplaced: 2 });
    let lost = server(KindCounts { lost: true, dropped: 0, unplaced: 0 });
    // A full pass whose frame dropped rows: logged, no reset.
    rig.script.draw_answers([Ok(dropped)]);
    let (_, made) = step(&mut rig, Some(E1));
    assert_eq!(made[..2], calls(&["open", "begin+"]));
    let (_, made) = step(&mut rig, Some(E1));
    assert_eq!(made[0], "begin", "no second reset after a full pass that left rows out");
    assert_eq!(rig.stats().resets, 0);
    // A pass that was not full and left rows out (case 4), then a full pass that lost the
    // kind (case 3, answered whatever the pass).
    rig.script.draw_answers([Ok(unplaced), Ok(lost)]);
    let (_, made) = step(&mut rig, Some(E1));
    assert_eq!(made[0], "begin");
    let (_, made) = step(&mut rig, Some(E1));
    assert_eq!(made[..2], calls(&["sink", "begin+"]), "case 4");
    let (_, made) = step(&mut rig, Some(E1));
    assert_eq!(made[..2], calls(&["sink", "begin+"]), "case 3, after a full pass");
    let (_, made) = step(&mut rig, Some(E1));
    assert_eq!(made[0], "begin");
    assert_eq!(rig.stats().resets, 2);
    rig.stop();
}

/// The bound on deferring, for a region: a frame defers while the ticks since the last
/// passes are at most `log_retention - CATCH_UP_CAP`, then is starved and hidden, its
/// passes run with no sink, and so is every frame after it until a region opens; a
/// starved frame whose `open` fails is hidden and counted torn as well.
fn starves(answer: Answer) {
    let setup = Setup { log_retention: Some(CATCH_UP_CAP + 2), ..Setup::new(2, false) };
    let mut rig = Rig::with(setup);
    rig.step(Some(E1));
    rig.script.take_calls();
    // Every region goes after the next submit.
    rig.script.submit_answers([Ok(Region::Wait(super::support::point(97)))]);
    rig.step(Some(E1));
    rig.settle();
    rig.script.take_calls();
    rig.script.open_answers(&[answer; 5]);
    let deferred = if answer == Answer::Wait { "waited" } else { "torn" };
    for frame in 0..2 {
        let (rendered, made) = step(&mut rig, Some(E1));
        let name = match rendered {
            Rendered::Waited(_) => "waited",
            Rendered::Torn => "torn",
            other => panic!("frame {frame} deferred, not {other:?}"),
        };
        assert_eq!(name, deferred);
        assert_eq!(made, calls(&["open"]));
    }
    // The third tick since the passes is past the bound: hidden, passes run with no sink.
    let before = rig.stats();
    let ticked = rig.shown().server().tick();
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(rendered, Rendered::Hidden);
    assert_eq!(made, calls(&["open", "present"]));
    assert_eq!(rig.shown().server().tick(), Tick(ticked.0 + 1));
    let after = rig.stats();
    assert_eq!(after.hidden, before.hidden + 1);
    let torn = u64::from(answer != Answer::Wait);
    assert_eq!(after.torn, before.torn + torn, "a starved frame whose open failed is torn");
    assert_eq!(after.waited, before.waited);
    // Starved still, with one tick since the passes: hidden while no region opens.
    let (rendered, made) = step(&mut rig, Some(E1));
    assert_eq!(rendered, Rendered::Hidden);
    assert_eq!(made, calls(&["open", "present"]));
    // A region: drawn, with a new sink.
    rig.script.with(|plan| plan.opens.clear());
    let (rendered, made) = step(&mut rig, Some(E1));
    drew(Some(rendered));
    assert_eq!(made[..3], calls(&["open", "sink", "begin+"]));
    rig.stop();
}

#[test]
fn a_frame_starved_of_a_region_is_hidden_past_the_bound() {
    starves(Answer::Wait);
}

#[test]
fn a_starved_frame_whose_open_fails_is_hidden_and_torn() {
    starves(Answer::Fail(ErrorKind::ResourceExhausted));
}

/// What the frame adds is no hashed state ("Determinism of a shown world"): the same game
/// built shown and unshown, stepped with the same frames through drawn, hidden, waited and
/// torn frames, hashes alike at every tick. Criterion 3(c) records the same under SOUR.
#[test]
fn a_shown_world_hashes_as_the_same_world_unshown() {
    let mut rig = Rig::new(2, false);
    let cooked = Arc::new(InMemory::new(REGION).expect("the content"));
    let mut frames = super::support::frames(&cooked);
    let mut plain = Some(
        build_server(&rig.executor, &GAME, WorldConfig::shown(), Start::Fresh, cooked)
            .expect("the unshown world"),
    );
    for step in 0..12_u64 {
        // Drawn, leaving no region; waited; hidden; torn at its draw.
        let rendered = match step % 4 {
            0 => {
                rig.script.submit_answers([Ok(Region::Wait(super::support::point(60 + step)))]);
                rig.step(Some(E1))
            }
            1 => {
                rig.script.open_answers(&[Answer::Wait]);
                rig.step(Some(E1))
            }
            2 => rig.step(None),
            _ => {
                rig.script.draw_answers([Err(Error::new(ErrorKind::Platform, "scripted"))]);
                rig.step(Some(E1))
            }
        };
        rig.settle();
        let expected = match step % 4 {
            0 => "drawn",
            1 => "waited",
            2 => "hidden",
            _ => "torn",
        };
        let name = match rendered {
            Rendered::Drew { .. } => "drawn",
            Rendered::Waited(_) => "waited",
            Rendered::Hidden => "hidden",
            Rendered::Torn => "torn",
            Rendered::Busy => "busy",
        };
        assert_eq!(name, expected, "step {step}");
        let server = plain.take().expect("the unshown world");
        let input = frames(server.tick()).expect("a frame");
        let stepped = bounded(&rig.executor, server.step(&rig.executor, input), "the plain step");
        let (server, _) = stepped.expect("the unshown step");
        let shown = rig.shown().server();
        assert_eq!(server.tick(), shown.tick());
        assert_eq!(server.hash().expect("hash"), shown.hash().expect("hash"), "step {step}");
        plain = Some(server);
    }
    rig.stop();
}
