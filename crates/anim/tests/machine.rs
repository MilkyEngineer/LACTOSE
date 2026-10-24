//! The state machine (`README.md`, "The state machine"; "Proof for milestone 1", the first
//! row): the transition order, one transition per tick, the entry tick's fields, the
//! trigger rule, `Finished`, a blend's weight and end, a blend ended by a machine with fewer
//! states, a transition into a state whose clip is missing, and the same hashes under three
//! layouts and one and four workers.

mod common;

use common::{
    Fixture, Library, MACHINE, Options, SKELETON, chain, clip, clip_ref, key, s, state, transition,
};
use lactose_anim::{AnimRig, AnimState, Condition, Phase, RootMotion, StateMachine};
use lactose_asset_api::{AssetRef, PinnedTable};
use lactose_ecs::WorldKind;
use lactose_sim::harness::{ChunkCapacity, layout_mode};

const IDLE: u16 = 0;
const WALK: u16 = 1;
const RUN: u16 = 2;
const JUMP: u16 = 3;

const IDLE_CLIP: AssetRef<lactose_anim::Clip> = clip_ref("machine/idle");
const WALK_CLIP: AssetRef<lactose_anim::Clip> = clip_ref("machine/walk");
const RUN_CLIP: AssetRef<lactose_anim::Clip> = clip_ref("machine/run");
const JUMP_CLIP: AssetRef<lactose_anim::Clip> = clip_ref("machine/jump");

const IDLE_TICKS: u32 = 60;
const WALK_TICKS: u32 = 40;
const RUN_TICKS: u32 = 20;
const JUMP_TICKS: u32 = 10;

/// The speed parameter the transitions compare.
const SPEED: u8 = 0;
/// The trigger that jumps from any state.
const JUMP_TRIGGER: u8 = 0;
/// The trigger that restarts the walk.
const RESTART_TRIGGER: u8 = 1;
/// A trigger no transition names.
const SPARE_TRIGGER: u8 = 5;

const QUARTER: Phase = Phase(1 << 30);
const WALK_BLEND: u16 = 4;
const LAND_BLEND: u16 = 6;

/// The machine of these tests, its transitions in this order:
///
/// 0. any state but the jump to the jump on the jump trigger;
/// 1. idle to walk above half a unit of speed, entering at a quarter, a four-tick blend;
/// 2. walk to run above two units, keeping the phase;
/// 3. the jump back to idle when finished, a six-tick blend;
/// 4. walk to walk (a restart) on the restart trigger, a two-tick blend;
/// 5. idle to run above half a unit, which transition 1 always shadows.
fn machine() -> StateMachine {
    let mut walk = transition(Some(IDLE), WALK, &[above(0.5)]);
    walk.blend_ticks = WALK_BLEND;
    walk.entry_phase = QUARTER;
    let mut run = transition(Some(WALK), RUN, &[above(2.0)]);
    run.keep_phase = true;
    let mut land = transition(Some(JUMP), IDLE, &[Condition::Finished]);
    land.blend_ticks = LAND_BLEND;
    let mut restart = transition(Some(WALK), WALK, &[Condition::Trigger(RESTART_TRIGGER)]);
    restart.blend_ticks = 2;
    StateMachine {
        states: vec![
            state("idle", IDLE_CLIP, 1.0),
            state("walk", WALK_CLIP, 1.0),
            state("run", RUN_CLIP, 1.0),
            state("jump", JUMP_CLIP, 1.0),
        ],
        transitions: vec![
            transition(None, JUMP, &[Condition::Trigger(JUMP_TRIGGER)]),
            walk,
            run,
            land,
            restart,
            transition(Some(IDLE), RUN, &[above(0.5)]),
        ],
        initial: IDLE,
    }
}

fn above(value: f64) -> Condition {
    Condition::Above { param: SPEED, value: s(value) }
}

fn library() -> Library {
    Library::new()
        .with(SKELETON, chain(2, |_| false))
        .with(MACHINE, machine())
        .with(IDLE_CLIP, clip(2, 4, IDLE_TICKS, true, 0.0))
        .with(WALK_CLIP, clip(2, 4, WALK_TICKS, true, 0.3))
        .with(RUN_CLIP, clip(2, 4, RUN_TICKS, true, 0.6))
        .with(JUMP_CLIP, clip(2, 5, JUMP_TICKS, false, 0.9))
}

fn rate(duration: u32) -> u32 {
    Phase::rate(duration, s(1.0))
}

fn at(duration: u32, ticks: u32) -> Phase {
    Phase(rate(duration).wrapping_mul(ticks))
}

const HERO: u64 = 1;

/// A server world with one character on the test machine, its assets pinned.
fn one_character() -> Fixture {
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library().pin());
    fixture.spawn(key(HERO), AnimRig::new(MACHINE, SKELETON));
    fixture
}

fn hero(fixture: &Fixture) -> AnimState {
    fixture.state(key(HERO))
}

#[test]
fn the_first_advance_enters_the_initial_state() {
    let mut fixture = one_character();
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(IDLE));
    assert!(row.entered() && row.entry_was_set());
    assert_eq!(row.ticks_in_state(), 1);
    assert_eq!(row.previous_phase(), Phase::ZERO);
    assert_eq!(row.phase(), at(IDLE_TICKS, 1));
    assert!(row.phase_crossed(Phase::ZERO), "the set entry phase is crossed on the entry tick");
    assert!(!row.finished());
    assert_eq!(row.blend(), None);

    fixture.step();
    let row = hero(&fixture);
    assert!(!row.entered() && !row.entry_was_set());
    assert_eq!(row.ticks_in_state(), 2);
    assert_eq!(row.previous_phase(), at(IDLE_TICKS, 1));
    assert_eq!(row.phase(), at(IDLE_TICKS, 2));
    assert!(!row.phase_crossed(Phase::ZERO));
}

#[test]
fn transitions_are_tried_in_the_machines_order() {
    let mut fixture = one_character();
    fixture.spawn(key(2), AnimRig::new(MACHINE, SKELETON));
    fixture.step();
    // Both: the jump trigger (transition 0) and the speed (transitions 1 and 5).
    fixture.set_params(key(HERO), |params| {
        params.values[usize::from(SPEED)] = s(1.0);
        params.trigger(JUMP_TRIGGER);
    });
    // The speed alone: transition 1 shadows transition 5.
    fixture.set_params(key(2), |params| params.values[usize::from(SPEED)] = s(1.0));
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(JUMP));
    assert_eq!(fixture.state(key(2)).state(), Some(WALK));
}

#[test]
fn one_transition_is_taken_per_tick() {
    let mut fixture = one_character();
    fixture.set_params(key(HERO), |params| params.values[usize::from(SPEED)] = s(3.0));
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(IDLE), "the entry advance tries no transition");
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(WALK), "idle to walk, not on to run");
    let walked = hero(&fixture).phase();
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(RUN), "walk to run on the next tick");
    assert!(row.entered() && !row.entry_was_set(), "a kept phase");
    assert_eq!(row.previous_phase(), walked, "the walk's phase before this tick's advance");
    assert_eq!(row.phase(), Phase(walked.0.wrapping_add(rate(RUN_TICKS))));
    assert!(!row.phase_crossed(walked), "a kept phase was passed by the tick that reached it");
    assert_eq!(row.blend(), None, "transition 2 has no blend");
}

#[test]
fn a_transitions_entry_tick_sets_its_fields() {
    let mut fixture = one_character();
    fixture.run(3);
    fixture.set_params(key(HERO), |params| params.values[usize::from(SPEED)] = s(1.0));
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(WALK));
    assert!(row.entered() && row.entry_was_set());
    assert_eq!(row.ticks_in_state(), 1);
    assert_eq!(row.previous_phase(), QUARTER);
    assert_eq!(row.phase(), Phase(QUARTER.0 + rate(WALK_TICKS)));
    assert!(row.phase_crossed(QUARTER), "the set entry phase");
    assert!(!row.finished());
    let blend = row.blend().unwrap();
    assert_eq!(blend.from, IDLE);
    // The outgoing idle had three ticks; it advanced a fourth by its own rate.
    assert_eq!(blend.phase, at(IDLE_TICKS, 4));
    assert_eq!(blend.weight, s(0.75));
}

#[test]
fn a_blend_weighs_down_per_tick_and_ends_at_its_length() {
    let mut fixture = one_character();
    fixture.step();
    fixture.set_params(key(HERO), |params| params.values[usize::from(SPEED)] = s(1.0));
    let mut weights = Vec::new();
    let mut phases = Vec::new();
    for _ in 0..u32::from(WALK_BLEND) + 2 {
        fixture.step();
        let row = hero(&fixture);
        assert_eq!(row.state(), Some(WALK));
        weights.push(row.blend().map(|blend| blend.weight));
        phases.push(row.blend().map(|blend| blend.phase));
    }
    assert_eq!(weights, [Some(s(0.75)), Some(s(0.5)), Some(s(0.25)), None, None, None]);
    let outgoing = |ticks| Some(at(IDLE_TICKS, ticks));
    assert_eq!(phases, [outgoing(2), outgoing(3), outgoing(4), None, None, None]);
    // The blend's end leaves the state playing on.
    assert_eq!(hero(&fixture).ticks_in_state(), u32::from(WALK_BLEND) + 2);
}

#[test]
fn a_restart_reenters_the_state_with_a_blend_between_its_two_phases() {
    let mut fixture = one_character();
    fixture.step();
    fixture.set_params(key(HERO), |params| params.values[usize::from(SPEED)] = s(1.0));
    fixture.run(10);
    let before = hero(&fixture).phase();
    fixture.set_params(key(HERO), |params| params.trigger(RESTART_TRIGGER));
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(WALK));
    assert!(row.entered() && row.entry_was_set());
    assert_eq!(row.previous_phase(), Phase::ZERO);
    let blend = row.blend().unwrap();
    assert_eq!((blend.from, blend.phase), (WALK, Phase(before.0 + rate(WALK_TICKS))));
    assert_eq!(fixture.params(key(HERO)).triggers, 0);
}

#[test]
fn a_trigger_waits_for_the_first_advance_that_tries_transitions() {
    let mut fixture = one_character();
    // Set before the character's first advance: the entry advance tries none.
    fixture.set_params(key(HERO), |params| params.trigger(JUMP_TRIGGER));
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(IDLE));
    assert_eq!(fixture.params(key(HERO)).triggers, 1 << JUMP_TRIGGER);
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(JUMP));
    assert_eq!(fixture.params(key(HERO)).triggers, 0, "consumed by the advance that took it");
}

#[test]
fn a_trigger_is_consumed_whether_or_not_a_transition_took_it() {
    let mut fixture = one_character();
    fixture.step();
    fixture.set_params(key(HERO), |params| {
        params.trigger(RESTART_TRIGGER);
        params.trigger(SPARE_TRIGGER);
    });
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(IDLE), "the restart does not apply in idle");
    assert_eq!(fixture.params(key(HERO)).triggers, 0);
    // A flag and a value are never written.
    fixture.set_params(key(HERO), |params| {
        params.set_flag(3, true);
        params.values[7] = s(9.0);
    });
    fixture.run(2);
    let params = fixture.params(key(HERO));
    assert!(params.flag(3));
    assert_eq!(params.values[7], s(9.0));
}

#[test]
fn a_hold_consumes_no_trigger() {
    // The machine missing (step 1).
    let mut fixture = one_character();
    fixture.step();
    fixture.assets(library().pin_without(&[MACHINE.id()]));
    fixture.set_params(key(HERO), |params| params.trigger(JUMP_TRIGGER));
    fixture.run(3);
    assert_eq!(fixture.params(key(HERO)).triggers, 1 << JUMP_TRIGGER);
    assert_eq!(hero(&fixture).state(), Some(IDLE));
    fixture.assets(library().pin());
    fixture.step();
    assert_eq!(hero(&fixture).state(), Some(JUMP));
    assert_eq!(fixture.params(key(HERO)).triggers, 0);

    // The state's clip missing (step 4).
    let mut fixture = fixture_without(&[IDLE_CLIP.id()]);
    fixture.set_params(key(HERO), |params| params.trigger(JUMP_TRIGGER));
    fixture.run(3);
    assert_eq!(fixture.params(key(HERO)).triggers, 1 << JUMP_TRIGGER);
    assert_eq!(hero(&fixture).state(), Some(IDLE));
}

fn fixture_without(missing: &[lactose_asset_api::AssetId]) -> Fixture {
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library().pin_without(missing));
    fixture.spawn(key(HERO), AnimRig::new(MACHINE, SKELETON));
    fixture
}

#[test]
fn finished_is_taken_one_tick_after_the_one_shot_reaches_its_end() {
    let mut fixture = one_character();
    fixture.step();
    fixture.set_params(key(HERO), |params| params.trigger(JUMP_TRIGGER));
    for tick in 1..JUMP_TICKS {
        fixture.step();
        let row = hero(&fixture);
        assert_eq!((row.state(), row.ticks_in_state()), (Some(JUMP), tick));
        assert!(row.phase() < Phase::END && !row.finished(), "tick {tick} of the jump");
    }
    fixture.step();
    let row = hero(&fixture);
    assert_eq!((row.state(), row.ticks_in_state()), (Some(JUMP), JUMP_TICKS));
    assert_eq!(row.phase(), Phase::END);
    assert!(row.finished());
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(IDLE), "landed the tick after the end");
    assert!(row.entered() && !row.finished());
    assert_eq!(row.blend().map(|blend| (blend.from, blend.phase)), Some((JUMP, Phase::END)));
}

#[test]
fn a_machine_with_fewer_states_ends_the_blend_and_reenters_a_lost_state() {
    let fewer = || {
        let mut fewer = machine();
        fewer.states.truncate(3);
        fewer.transitions.retain(|t| t.to != JUMP && t.from != Some(JUMP));
        library().with(MACHINE, fewer).pin()
    };
    // Mid-blend from the jump into idle.
    let mut fixture = one_character();
    fixture.step();
    fixture.set_params(key(HERO), |params| params.trigger(JUMP_TRIGGER));
    fixture.run(JUMP_TICKS + 2);
    let row = hero(&fixture);
    assert_eq!((row.state(), row.blend().map(|blend| blend.from)), (Some(IDLE), Some(JUMP)));
    let phase = row.phase();
    fixture.assets(fewer());
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(IDLE));
    assert_eq!(row.blend(), None, "the outgoing state is past the states");
    assert!(!row.entered());
    assert_eq!(row.ticks_in_state(), 3, "landed two ticks before");
    assert_eq!(row.phase(), Phase(phase.0 + rate(IDLE_TICKS)), "the state plays on");

    // In the jump itself: step 2 re-enters the initial state.
    let mut fixture = one_character();
    fixture.step();
    fixture.set_params(key(HERO), |params| params.trigger(JUMP_TRIGGER));
    fixture.run(3);
    assert_eq!(hero(&fixture).state(), Some(JUMP));
    fixture.assets(fewer());
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(IDLE));
    assert!(row.entered() && row.entry_was_set());
    assert_eq!(row.ticks_in_state(), 1);
}

/// The rule for a transition into a state whose clip is missing, with a set entry phase:
/// the transition stands, the triggers are consumed, and the tick holds with the entry
/// reported once at `ticks_in_state` zero; the held ticks report nothing; the first advance
/// once the clip is there passes `(entry, phase]`.
#[test]
fn a_transition_into_a_missing_clip_reports_a_set_entry_once() {
    let mut fixture = fixture_without(&[WALK_CLIP.id()]);
    fixture.step();
    fixture.set_params(key(HERO), |params| {
        params.values[usize::from(SPEED)] = s(1.0);
        params.trigger(SPARE_TRIGGER);
    });
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(WALK), "the transition is taken");
    assert!(row.entered() && row.entry_was_set());
    assert_eq!(row.ticks_in_state(), 0);
    assert_eq!((row.previous_phase(), row.phase()), (QUARTER, QUARTER));
    assert!(row.phase_crossed(QUARTER), "the entry phase, reported on the entry tick");
    assert_eq!(row.blend().map(|blend| blend.from), Some(IDLE), "the blend as taken");
    assert_eq!(fixture.params(key(HERO)).triggers, 0, "consumed by the advance that tried");
    assert_eq!(fixture.motion(key(HERO)), RootMotion::ZERO);

    fixture.set_params(key(HERO), |params| params.trigger(SPARE_TRIGGER));
    for _ in 0..3 {
        fixture.step();
        let row = hero(&fixture);
        assert_eq!(row.state(), Some(WALK));
        assert!(!row.entered() && !row.entry_was_set(), "a held tick reports no entry");
        assert!(!row.phase_crossed(QUARTER), "nor the entry phase again");
        assert_eq!(row.ticks_in_state(), 0);
        assert_eq!(fixture.params(key(HERO)).triggers, 1 << SPARE_TRIGGER, "a hold");
    }
    fixture.set_params(key(HERO), |params| params.triggers = 0);
    fixture.assets(library().pin());
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(WALK));
    assert!(!row.entered());
    assert_eq!(row.ticks_in_state(), 1);
    assert_eq!(row.previous_phase(), QUARTER);
    assert_eq!(row.phase(), Phase(QUARTER.0 + rate(WALK_TICKS)));
    assert!(!row.phase_crossed(QUARTER), "the entry phase is not reported again");
}

/// The same rule with a kept phase: the entry is reported once and no phase with it.
#[test]
fn a_transition_into_a_missing_clip_reports_a_kept_entry_once() {
    let mut fixture = fixture_without(&[RUN_CLIP.id()]);
    fixture.step();
    fixture.set_params(key(HERO), |params| params.values[usize::from(SPEED)] = s(1.0));
    fixture.run(3);
    assert_eq!(hero(&fixture).state(), Some(WALK));
    let walked = hero(&fixture).phase();
    fixture.set_params(key(HERO), |params| {
        params.values[usize::from(SPEED)] = s(3.0);
        params.trigger(SPARE_TRIGGER);
    });
    fixture.step();
    let row = hero(&fixture);
    assert_eq!(row.state(), Some(RUN));
    assert!(row.entered() && !row.entry_was_set());
    assert_eq!(row.ticks_in_state(), 0);
    assert_eq!((row.previous_phase(), row.phase()), (walked, walked));
    assert!(!row.phase_crossed(walked), "a kept phase is not crossed");
    assert_eq!(fixture.params(key(HERO)).triggers, 0);

    fixture.run(2);
    let row = hero(&fixture);
    assert!(!row.entered() && !row.phase_crossed(walked));
    assert_eq!(row.ticks_in_state(), 0);
    fixture.assets(library().pin());
    fixture.step();
    let row = hero(&fixture);
    assert_eq!((row.state(), row.ticks_in_state()), (Some(RUN), 1));
    assert_eq!(row.previous_phase(), walked);
    assert_eq!(row.phase(), Phase(walked.0 + rate(RUN_TICKS)));
    assert!(fixture.log().events.is_empty(), "the test clips carry no events");
}

/// What every character of the hash run does at `tick`: a speed ramp, jumps and restarts
/// at ticks of its own, so the machine walks every transition.
fn drive(fixture: &mut Fixture, characters: u64, tick: u32) {
    for n in 0..characters {
        let phase = u32::try_from(n).unwrap() * 7;
        let speed = f64::from((tick + phase) % 90) / 30.0;
        fixture.set_params(key(10 + n), |params| {
            params.values[usize::from(SPEED)] = s(speed);
            if (tick + phase).is_multiple_of(37) {
                params.trigger(JUMP_TRIGGER);
            }
            if (tick + phase).is_multiple_of(23) {
                params.trigger(RESTART_TRIGGER);
            }
        });
    }
}

fn hashes(layout: lactose_ecs::LayoutConfig, workers: u32) -> Vec<(u64, Vec<AnimState>)> {
    const CHARACTERS: u64 = 24;
    let mut options = Options::new(WorldKind::Server);
    options.layout = layout;
    options.workers = workers;
    let mut fixture = options.build();
    let table: PinnedTable = library().pin();
    fixture.assets(table);
    for n in 0..CHARACTERS {
        fixture.spawn(key(10 + n), AnimRig::new(MACHINE, SKELETON));
    }
    (0..200)
        .map(|tick| {
            drive(&mut fixture, CHARACTERS, tick);
            fixture.step();
            let states = (0..CHARACTERS).map(|n| fixture.state(key(10 + n))).collect();
            (fixture.hash(), states)
        })
        .collect()
}

#[test]
fn hashes_are_equal_under_three_layouts_and_one_and_four_workers() {
    let layouts = [
        lactose_ecs::LayoutConfig::default(),
        layout_mode(ChunkCapacity::Rows(3), 7).unwrap(),
        layout_mode(ChunkCapacity::Bytes(16384), 11).unwrap(),
    ];
    let reference = hashes(layouts[0], 1);
    let states: std::collections::BTreeSet<_> =
        reference.iter().flat_map(|(_, rows)| rows.iter().filter_map(AnimState::state)).collect();
    assert_eq!(states.len(), 4, "the run visits every state");
    for layout in layouts {
        for workers in [1, 4] {
            assert!(hashes(layout, workers) == reference, "{layout:?}, {workers} workers");
        }
    }
}
