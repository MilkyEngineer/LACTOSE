//! Phase and crossings (`README.md`, "Phase and crossings"; "Proof for milestone 1", the
//! phase row): `rate`'s cases and its Q16.16 path against an f32 path, what the ceiling
//! gives, `advanced` and `passed`, and `phase_crossed` once per pass at every rate, on a
//! set entry phase and not on a kept one, never on a held tick.

#![forbid(unsafe_code)]

mod common;

use common::{
    Fixture, Library, MACHINE, SKELETON, chain, clip, clip_ref, key, s, state, transition,
};
use lactose_anim::{AnimRig, Condition, Phase, StateMachine};
use lactose_asset_api::AssetRef;
use lactose_ecs::WorldKind;
use lactose_math::Scalar;

const UNITS: u128 = 1 << 32;

#[test]
fn rate_is_zero_for_a_speed_it_cannot_take_and_for_a_zero_duration() {
    for speed in [
        Scalar::ZERO,
        s(-1.0),
        s(-0.0),
        s(f64::NAN),
        s(f64::INFINITY),
        s(f64::NEG_INFINITY),
        s(1.0 / 131_072.0),
    ] {
        assert_eq!(Phase::rate(60, speed), 0, "{speed:?}");
    }
    assert_eq!(Phase::rate(0, s(1.0)), 0);
    // One Q16.16 unit: the smallest speed that moves.
    assert_eq!(Phase::rate(1, s(1.0 / 65_536.0)), 1 << 16);
    assert_eq!(Phase::rate(3, s(1.0 / 65_536.0)), 21_846, "ceil(2^16 / 3)");
}

#[test]
fn rate_clamps_a_speed_of_a_clip_per_tick_or_more() {
    assert_eq!(Phase::rate(60, s(60.0)), u32::MAX);
    assert_eq!(Phase::rate(60, s(1e12)), u32::MAX);
    assert_eq!(Phase::rate(1, s(1.0)), u32::MAX);
    assert_eq!(Phase::rate(60, s(59.0)), 4_223_384_508, "ceil(2^32 * 59 / 60)");
}

/// The same rate computed in `f32` as a hand calculation would: the speed times `2^16` in
/// `f32`, truncated, then the ceiling in integers.
fn f32_rate(duration: u32, speed: f32) -> u32 {
    #[expect(clippy::cast_possible_truncation, reason = "the hand path truncates as `to_i64` does")]
    let q = (speed * 65_536.0) as i64;
    let q = u128::try_from(q.clamp(0, i64::from(u32::MAX))).unwrap();
    let units = (UNITS * q).div_ceil(u128::from(duration) << 16);
    u32::try_from(units).unwrap_or(u32::MAX)
}

#[test]
fn rates_q16_16_path_matches_an_f32_path_for_durations_1_to_600() {
    let speeds: [f32; 10] =
        [1.0, 0.5, 0.75, 1.5, 2.0, 0.1, 1.0 / 3.0, 0.016_666_668, 2.718_281_7, 1.0 / 65_536.0];
    let mut checked = 0;
    for duration in 1..=600 {
        for speed in speeds {
            let scalar = Scalar::from_f32(speed);
            assert_eq!(
                Phase::rate(duration, scalar),
                f32_rate(duration, speed),
                "duration {duration}, speed {speed}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 6_000);
}

/// The phase after `ticks` ticks at `rate` from zero, unwrapped.
fn unwrapped(rate: u32, ticks: u32) -> u128 {
    u128::from(rate) * u128::from(ticks)
}

#[test]
fn the_ceiling_gives_the_ticks_the_contract_promises() {
    for d in 1..=600u32 {
        let rate = Phase::rate(d, s(1.0));
        // A one-shot of `d` ticks holds `END` on tick `d` and not before.
        let mut phase = Phase::ZERO;
        for tick in 1..=d {
            phase = phase.advanced(rate, false);
            assert_eq!(phase == Phase::END, tick == d, "one-shot of {d}, tick {tick}");
        }
        // A loop of `d` ticks wraps on tick `d` (its first cycle); a loop of one tick at
        // speed one is the clamp's case, a clip per tick, and never wraps.
        assert!(
            d == 1 || unwrapped(rate, d - 1) < UNITS && unwrapped(rate, d) >= UNITS,
            "loop of {d}"
        );
        // `from_fraction(f, d)` is crossed on tick `f`, and on no other tick of the cycle.
        for f in 1..d {
            let mark = Phase::from_fraction(f, d);
            let crossed: Vec<u32> = (1..=d)
                .filter(|&tick| {
                    let from = Phase(rate.wrapping_mul(tick - 1));
                    let to = Phase(rate.wrapping_mul(tick));
                    Phase::passed(from, to, mark)
                })
                .collect();
            assert_eq!(crossed, [f], "d {d}, f {f}");
        }
        // The `m`-th wrap comes one tick early, on tick `m * d - 1`, and every earlier one
        // on time; never sooner than `2^32 / (d - 1)` ticks in.
        if d > 1 {
            let lead = u128::from(d) * u128::from(rate) - UNITS;
            if lead == 0 {
                continue;
            }
            let m = u128::from(rate).div_ceil(lead);
            let wraps = |tick: u128| tick * u128::from(rate) / UNITS;
            let early = m * u128::from(d) - 1;
            assert_eq!(wraps(early), m, "d {d}: wrap {m} on tick {early}");
            assert_eq!(wraps(early - 1), m - 1, "d {d}: not before");
            assert_eq!(wraps((m - 1) * u128::from(d) - 1), m - 2, "d {d}: the wrap before on time");
            assert!(early >= UNITS / u128::from(d - 1), "d {d}: early wrap at {early}");
        }
    }
}

#[test]
fn advanced_wraps_a_loop_and_saturates_a_one_shot() {
    assert_eq!(Phase(u32::MAX - 1).advanced(3, true), Phase(1));
    assert_eq!(Phase(u32::MAX - 1).advanced(3, false), Phase::END);
    assert_eq!(Phase::END.advanced(1, false), Phase::END);
    assert_eq!(Phase::HALF.advanced(0, true), Phase::HALF);
    assert_eq!(Phase(5).advanced(7, false), Phase(12));
}

#[test]
fn passed_is_half_open_and_crosses_the_wrap() {
    // `(from, to]`.
    assert!(!Phase::passed(Phase(10), Phase(20), Phase(10)));
    assert!(Phase::passed(Phase(10), Phase(20), Phase(11)));
    assert!(Phase::passed(Phase(10), Phase(20), Phase(20)));
    assert!(!Phase::passed(Phase(10), Phase(20), Phase(21)));
    // Equal phases pass nothing.
    assert!(!Phase::passed(Phase::HALF, Phase::HALF, Phase::HALF));
    // At `END`.
    assert!(Phase::passed(Phase(u32::MAX - 5), Phase::END, Phase::END));
    assert!(!Phase::passed(Phase::END, Phase::END, Phase::END));
    // Across the wrap: `(from, END] ∪ [ZERO, to]`.
    let (from, to) = (Phase(u32::MAX - 10), Phase(10));
    for (mark, expected) in [
        (Phase(u32::MAX - 10), false),
        (Phase(u32::MAX - 9), true),
        (Phase::END, true),
        (Phase::ZERO, true),
        (Phase(10), true),
        (Phase(11), false),
        (Phase::HALF, false),
    ] {
        assert_eq!(Phase::passed(from, to, mark), expected, "{mark:?}");
    }
}

const LOOP_CLIP: AssetRef<lactose_anim::Clip> = clip_ref("phase/loop");
const RUN_CLIP: AssetRef<lactose_anim::Clip> = clip_ref("phase/run");
const LOOP_TICKS: u32 = 32;

/// One looping state at the speed of parameter zero, and a run it leaves for when the
/// phase is at or past half, keeping the phase.
fn library() -> Library {
    let mut walk = state("walk", LOOP_CLIP, 1.0);
    walk.speed_param = Some(0);
    let mut run = transition(Some(0), 1, &[Condition::PhaseAtLeast(Phase::HALF)]);
    run.keep_phase = true;
    let machine = StateMachine {
        states: vec![walk, state("run", RUN_CLIP, 1.0)],
        transitions: vec![run],
        initial: 0,
    };
    Library::new()
        .with(SKELETON, chain(2, |_| false))
        .with(MACHINE, machine)
        .with(LOOP_CLIP, clip(2, 4, LOOP_TICKS, true, 0.0))
        .with(RUN_CLIP, clip(2, 4, 20, true, 0.0))
}

/// A machine of the loop alone, so the state never leaves it.
fn loop_only() -> Library {
    let mut walk = state("walk", LOOP_CLIP, 1.0);
    walk.speed_param = Some(0);
    let machine = StateMachine { states: vec![walk], transitions: Vec::new(), initial: 0 };
    library().with(MACHINE, machine)
}

#[test]
fn phase_crossed_is_true_once_per_pass_at_every_rate() {
    const TICKS: u32 = 400;
    let speeds = [0.0, 0.25, 0.5, 1.0, 1.7, 3.0, 7.5, 31.0];
    let marks = [Phase::ZERO, Phase(1), Phase::HALF, Phase::from_fraction(3, 7), Phase::END];
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(loop_only().pin());
    for (n, speed) in speeds.iter().enumerate() {
        let character = key(n as u64 + 1);
        fixture.spawn(character, AnimRig::new(MACHINE, SKELETON));
        fixture.set_params(character, |params| params.values[0] = s(*speed));
    }
    let mut counts = vec![[0u128; 5]; speeds.len()];
    for _ in 0..TICKS {
        fixture.step();
        for (n, count) in counts.iter_mut().enumerate() {
            let row = fixture.state(key(n as u64 + 1));
            for (slot, mark) in marks.iter().enumerate() {
                count[slot] += u128::from(row.phase_crossed(*mark));
            }
        }
    }
    for (n, speed) in speeds.iter().enumerate() {
        let rate = Phase::rate(LOOP_TICKS, s(*speed));
        // The first tick enters at zero and advances; the phase has moved `TICKS * rate`.
        let total = unwrapped(rate, TICKS);
        for (slot, mark) in marks.iter().enumerate() {
            let mark = u128::from(mark.0);
            // Passes over `mark + k * 2^32` in `(0, total]`, and the entry at zero.
            let expected = match mark {
                0 => total / UNITS + 1,
                _ if total >= mark => (total - mark) / UNITS + 1,
                _ => 0,
            };
            assert_eq!(counts[n][slot], expected, "speed {speed}, mark {mark}");
        }
    }
}

#[test]
fn a_kept_entry_phase_is_not_crossed_again() {
    // The walk lands exactly on half at tick 16 (rate `2^27`), and the run is taken the
    // tick after, keeping the phase: the footstep at half plays once.
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library().pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.set_params(key(1), |params| params.values[0] = s(1.0));
    let mut crossed = Vec::new();
    for tick in 1..=20 {
        fixture.step();
        let row = fixture.state(key(1));
        if row.phase_crossed(Phase::HALF) {
            crossed.push(tick);
        }
        if tick == 16 {
            assert_eq!((row.state(), row.phase()), (Some(0), Phase::HALF));
        }
        if tick == 17 {
            assert_eq!(row.state(), Some(1));
            assert!(row.entered() && !row.entry_was_set());
            assert_eq!(row.previous_phase(), Phase::HALF);
        }
    }
    assert_eq!(crossed, [16]);
}

#[test]
fn a_held_tick_crosses_nothing() {
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(loop_only().pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.set_params(key(1), |params| params.values[0] = s(1.0));
    fixture.run(16);
    assert!(fixture.state(key(1)).phase_crossed(Phase::HALF));
    fixture.assets(loop_only().pin_without(&[LOOP_CLIP.id()]));
    fixture.step();
    let row = fixture.state(key(1));
    assert_eq!(row.phase(), Phase::HALF, "the row keeps the last advance's phases");
    assert!(!row.phase_crossed(Phase::HALF) && !row.phase_crossed(Phase(1)));
    // Rate zero: the phases are equal and nothing is crossed either.
    fixture.assets(loop_only().pin());
    fixture.set_params(key(1), |params| params.values[0] = Scalar::ZERO);
    fixture.step();
    let row = fixture.state(key(1));
    assert_eq!((row.previous_phase(), row.phase()), (Phase::HALF, Phase::HALF));
    assert!(!row.phase_crossed(Phase::HALF));
    // The hold is over: the next advance crosses what it passes again.
    fixture.set_params(key(1), |params| params.values[0] = s(1.0));
    fixture.step();
    let row = fixture.state(key(1));
    assert!(row.phase() > Phase::HALF);
    assert!(row.phase_crossed(Phase(Phase::HALF.0 + 1)) && row.phase_crossed(row.phase()));
    assert!(!row.phase_crossed(Phase::HALF));
}

#[test]
fn an_entry_whose_clip_is_missing_reports_its_set_phase_once() {
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(loop_only().pin_without(&[LOOP_CLIP.id()]));
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.set_params(key(1), |params| params.values[0] = s(1.0));
    let mut crossed = Vec::new();
    for tick in 1..=4 {
        if tick == 3 {
            fixture.assets(loop_only().pin());
        }
        fixture.step();
        let row = fixture.state(key(1));
        crossed.push((tick, row.entered(), row.phase_crossed(Phase::ZERO), row.ticks_in_state()));
    }
    assert_eq!(
        crossed,
        [(1, true, true, 0), (2, false, false, 0), (3, false, false, 1), (4, false, false, 2)]
    );
}
