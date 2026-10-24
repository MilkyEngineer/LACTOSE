//! Authored clip events (`README.md`, "The state machine", step 9; "Proof for milestone
//! 1", the events row): one `AnimEvent` per pass in the order passed, across a wrap
//! included, to the character's key, read by a later system; none from the outgoing clip.

#![forbid(unsafe_code)]

mod common;

use common::{
    Fixture, Library, MACHINE, SKELETON, chain, clip, clip_ref, key, s, state, transition,
    with_events,
};
use lactose_anim::{
    AnimEvent, AnimEventKey, AnimRig, AnimState, Clip, Condition, Phase, StateMachine,
};
use lactose_asset_api::AssetRef;
use lactose_ecs::WorldKind;

const EVENTS_CLIP: AssetRef<Clip> = clip_ref("events/loop");
const QUIET_CLIP: AssetRef<Clip> = clip_ref("events/quiet");
const LOOP_TICKS: u32 = 10;

const MARKS: [Phase; 6] =
    [Phase::ZERO, Phase(1_000), Phase::HALF, Phase::HALF, Phase(u32::MAX - 1_000), Phase::END];

const AWAY: u8 = 0;
const BACK: u8 = 1;

/// State 0 plays the clip with events at the speed of parameter zero; state 1 a clip
/// without events. Away blends out of the events over five ticks; back enters at half.
fn library() -> Library {
    let mut loud = state("loud", EVENTS_CLIP, 1.0);
    loud.speed_param = Some(0);
    let mut away = transition(Some(0), 1, &[Condition::Trigger(AWAY)]);
    away.blend_ticks = 5;
    let mut back = transition(Some(1), 0, &[Condition::Trigger(BACK)]);
    back.entry_phase = Phase::HALF;
    let machine = StateMachine {
        states: vec![loud, state("quiet", QUIET_CLIP, 1.0)],
        transitions: vec![away, back],
        initial: 0,
    };
    Library::new()
        .with(SKELETON, chain(2, |_| false))
        .with(MACHINE, machine)
        .with(EVENTS_CLIP, with_events(clip(2, 4, LOOP_TICKS, true, 0.0), &MARKS))
        .with(QUIET_CLIP, clip(2, 4, LOOP_TICKS, true, 0.5))
}

/// The events an advance of `row` passed, by an independent statement of the rule: a mark
/// is passed when its distance from the previous phase, around the clip, is above zero and
/// at most the advance's (or zero, at a set entry phase); in order of that distance, then
/// in the clip's order.
fn expected(target: u64, row: &AnimState) -> Vec<AnimEvent> {
    if row.state() != Some(0) {
        return Vec::new();
    }
    let (from, to) = (row.previous_phase().0, row.phase().0);
    let advance = to.wrapping_sub(from);
    let mut passed: Vec<(u32, usize)> = MARKS
        .iter()
        .enumerate()
        .filter_map(|(index, mark)| {
            let distance = mark.0.wrapping_sub(from);
            let entry = distance == 0 && row.entry_was_set();
            (entry || (distance > 0 && distance <= advance)).then_some((distance, index))
        })
        .collect();
    passed.sort_unstable();
    passed
        .into_iter()
        .map(|(_, index)| AnimEvent {
            target: key(target),
            event: AnimEventKey(index as u64 + 1),
            state: 0,
            phase: MARKS[index],
        })
        .collect()
}

#[test]
fn events_are_emitted_once_per_pass_in_the_order_passed() {
    const CHARACTERS: u64 = 4;
    let speeds = [1.0, 2.3, 0.37, 9.5];
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library().pin());
    for n in 0..CHARACTERS {
        fixture.spawn(key(n + 1), AnimRig::new(MACHINE, SKELETON));
        fixture.set_params(key(n + 1), |params| params.values[0] = s(speeds[n as usize]));
    }
    let mut total = 0;
    let mut wraps_with_events = 0;
    for tick in 1..=120u32 {
        // Character one goes quiet for a while, blending out of the loud clip.
        if tick == 30 {
            fixture.set_params(key(1), |params| params.trigger(AWAY));
        }
        if tick == 50 {
            fixture.set_params(key(1), |params| params.trigger(BACK));
        }
        let now = fixture.tick();
        fixture.step();
        let log = fixture.take_events();
        for n in 1..=CHARACTERS {
            let row = fixture.state(key(n));
            let mine: Vec<AnimEvent> =
                log.iter().filter(|(_, e)| e.target == key(n)).map(|&(_, e)| e).collect();
            assert!(log.iter().all(|&(at, _)| at == now + 1), "emitted in the tick that ran");
            let want = expected(n, &row);
            assert_eq!(mine, want, "tick {tick}, character {n}");
            assert_eq!(fixture.seen(key(n)).events as usize, want.len(), "read by a later system");
            total += want.len();
            let wrapped = row.phase() < row.previous_phase();
            if wrapped
                && want.first().is_some_and(|e| e.phase > row.previous_phase())
                && want.last().is_some_and(|e| e.phase <= row.phase())
            {
                wraps_with_events += 1;
            }
        }
        assert_eq!(
            log.len(),
            (1..=CHARACTERS).map(|n| expected(n, &fixture.state(key(n))).len()).sum::<usize>()
        );
    }
    assert!(total > 300, "{total} events");
    assert!(wraps_with_events > 10, "{wraps_with_events} wraps passed events on both sides");
}

#[test]
fn an_outgoing_clip_emits_nothing() {
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library().pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.set_params(key(1), |params| params.values[0] = s(2.0));
    fixture.run(3);
    fixture.set_params(key(1), |params| params.trigger(AWAY));
    fixture.take_events();
    for _ in 0..5 {
        fixture.step();
        let row = fixture.state(key(1));
        assert_eq!(row.state(), Some(1));
        assert!(fixture.take_events().is_empty(), "the blend's outgoing clip passes marks");
    }
    // The outgoing phase did advance across the clip's marks.
    let row = fixture.state(key(1));
    assert_eq!(row.ticks_in_state(), 5);
}

#[test]
fn the_entry_at_a_set_phase_emits_the_events_there_first() {
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library().pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.set_params(key(1), |params| params.values[0] = s(1.0));
    fixture.step();
    let events: Vec<Phase> = fixture.take_events().iter().map(|(_, e)| e.phase).collect();
    assert_eq!(events, [Phase::ZERO, Phase(1_000)], "the entry at zero, then its advance");
    // Quiet, then back at half: both events at half on the entry tick, in the clip's order.
    fixture.set_params(key(1), |params| params.trigger(AWAY));
    fixture.step();
    fixture.set_params(key(1), |params| params.trigger(BACK));
    fixture.take_events();
    fixture.step();
    let events: Vec<(Phase, AnimEventKey)> =
        fixture.take_events().iter().map(|(_, e)| (e.phase, e.event)).collect();
    assert_eq!(events, [(Phase::HALF, AnimEventKey(3)), (Phase::HALF, AnimEventKey(4))]);
}
