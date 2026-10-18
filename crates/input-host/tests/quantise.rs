//! The raw sample and the quantiser against the README ("The raw sample", "Quantisation"):
//! the frame start and the combining rules, the wrapped yaw, every quantised field and its
//! zeros for non-finite input, the edges once per tick, and the latch.

#![forbid(unsafe_code)]

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use lactose_input::{AXIS_SCALE, Action, Actions, Axis2, Pitch, PlayerInput, UNITS_PER_TURN, Yaw};
use lactose_input_host::{Quantiser, RawSample};

const JUMP: Actions = Actions::EMPTY.with(Action::Jump);
const SPRINT: Actions = Actions::EMPTY.with(Action::Sprint);
const THROW: Actions = Actions::EMPTY.with(Action::Throw);

/// What a fresh quantiser makes of `raw`.
fn quantise(raw: RawSample) -> PlayerInput {
    Quantiser::new().next(&mut { raw })
}

fn movement(x: f32, y: f32) -> Axis2 {
    quantise(RawSample { movement_x: x, movement_y: y, ..RawSample::IDLE }).movement
}

fn yaw(radians: f32) -> u16 {
    quantise(RawSample { yaw: radians, ..RawSample::IDLE }).yaw.0
}

fn pitch(radians: f32) -> i16 {
    quantise(RawSample { pitch: radians, ..RawSample::IDLE }).pitch.0
}

/// `x² + y² - AXIS_SCALE²` of a quantised vector, in integers.
fn excess(movement: Axis2) -> i64 {
    let (x, y, scale) = (i64::from(movement.x), i64::from(movement.y), i64::from(AXIS_SCALE));
    x * x + y * y - scale * scale
}

/// The distance between two yaws in units, around the turn.
fn yaw_distance(a: u16, b: u16) -> u16 {
    let d = a.wrapping_sub(b);
    d.min(d.wrapping_neg())
}

/// Values every float field of a raw sample is swept over: in range, at and past the
/// edges, huge, tiny, subnormal, signed zeros and the non-finite ones.
const SWEEP: [f32; 24] = [
    0.0,
    -0.0,
    1e-40,
    f32::MIN_POSITIVE,
    0.1,
    -0.3,
    0.5,
    -0.5,
    core::f32::consts::FRAC_1_SQRT_2,
    1.0,
    -1.0,
    1.000_000_1,
    FRAC_PI_2,
    -2.0,
    PI,
    -PI,
    TAU + 0.1,
    1e6,
    -1e30,
    f32::MAX,
    -f32::MAX,
    f32::NAN,
    f32::INFINITY,
    f32::NEG_INFINITY,
];

// ---- Quantisation ----

#[test]
fn a_sweep_of_raw_samples_passes_validate() {
    let mut count = 0u32;
    for x in SWEEP {
        for y in SWEEP {
            for angle in SWEEP {
                let raw = RawSample {
                    movement_x: x,
                    movement_y: y,
                    yaw: angle,
                    pitch: angle,
                    held: SPRINT,
                    latched: JUMP,
                };
                let input = quantise(raw);
                input.validate().unwrap_or_else(|e| panic!("{raw:?} gives {input:?}: {e}"));
                let longer = |c: f32| c.is_finite() && c.abs() >= 1.0;
                if longer(x) || longer(y) {
                    assert!(excess(input.movement).abs() <= 2 * i64::from(AXIS_SCALE), "{raw:?}");
                }
                count += 1;
            }
        }
    }
    assert_eq!(count, 24 * 24 * 24);
}

/// Every direction at lengths about one, beyond it and far beyond: accepted, and every
/// input at least one long quantised to unit length within the rounding.
#[test]
fn every_direction_quantises_into_the_disc() {
    let directions = if cfg!(miri) { 500 } else { 50_000 };
    let scale = i64::from(AXIS_SCALE);
    for k in 0..directions {
        let angle = f64::from(k) / f64::from(directions) * core::f64::consts::TAU;
        let (x, y) = (angle.cos() as f32, angle.sin() as f32);
        for length in [0.5f32, 0.999_999_9, 1.0, 1.000_000_1, 1.5, 2.0, 1e30] {
            let quantised = movement(x * length, y * length);
            PlayerInput { movement: quantised, ..PlayerInput::IDLE }.validate().unwrap();
            if length >= 1.0 {
                assert!(excess(quantised).abs() <= 2 * scale, "{k} {length}: {quantised:?}");
            }
        }
    }
}

#[test]
fn a_vector_of_length_two_quantises_to_length_one() {
    assert_eq!(movement(2.0, 0.0), Axis2 { x: AXIS_SCALE, y: 0 });
    assert_eq!(movement(0.0, -2.0), Axis2 { x: 0, y: -AXIS_SCALE });
    assert_eq!(movement(f32::MAX, 0.0), Axis2 { x: AXIS_SCALE, y: 0 });
    let diagonal = movement(core::f32::consts::SQRT_2, core::f32::consts::SQRT_2);
    assert_eq!(diagonal.x, diagonal.y);
    assert!(excess(diagonal).abs() <= 2 * i64::from(AXIS_SCALE), "{diagonal:?}");
    // A huge finite stick keeps its direction: no square overflows.
    assert_eq!(movement(f32::MAX, f32::MAX), diagonal);
    assert_eq!(movement(-f32::MAX, f32::MAX), Axis2 { x: -diagonal.x, y: diagonal.y });
    // Inside the disc nothing is scaled: rounded to nearest, ties away from zero.
    assert_eq!(movement(1.0, 0.0), Axis2 { x: AXIS_SCALE, y: 0 });
    assert_eq!(movement(0.5, -0.5), Axis2 { x: 16384, y: -16384 });
    assert_eq!(movement(0.3, 0.4), Axis2 { x: 9830, y: 13107 });
    assert_eq!(movement(1e-40, -1e-40), Axis2 { x: 0, y: 0 });
}

#[test]
fn whole_turns_of_yaw_quantise_alike() {
    for radians in [0.1f32, -0.1, 1.0, -2.5, 3.0] {
        let units = yaw(radians);
        assert_eq!(yaw(radians + TAU), units, "{radians} + TAU");
        assert_eq!(yaw(radians - TAU), units, "{radians} - TAU");
        assert_eq!(yaw(radians + 4.0 * TAU), units, "{radians} + 4 TAU");
    }
    assert_eq!(yaw(0.0), 0);
    assert_eq!(yaw(-0.0), 0);
    assert_eq!(yaw(FRAC_PI_2), 16384);
    assert_eq!(yaw(-FRAC_PI_2), 49152);
    assert_eq!(yaw(PI), 32768);
    assert_eq!(yaw(-PI), 32768);
    // 0.1 rad is 1043.04 units; the largest `f32` below a turn rounds up to a turn, which
    // is zero.
    assert_eq!(yaw(0.1), 1043);
    assert_eq!(yaw(TAU.next_down()), 0);
    assert_eq!(yaw(-1e-6), 0);
    assert_eq!(yaw(-1e-3), u16::MAX - 9);
}

#[test]
fn pitch_beyond_a_quarter_turn_clamps() {
    for radians in [FRAC_PI_2, 1.6, 2.0, 100.0, f32::MAX, f32::INFINITY] {
        assert_eq!(pitch(radians), Pitch::LIMIT, "{radians}");
        assert_eq!(pitch(-radians), -Pitch::LIMIT, "-{radians}");
    }
    assert_eq!(pitch(0.0), 0);
    assert_eq!(pitch(0.5), 5215);
    assert_eq!(pitch(-0.5), -5215);
    assert_eq!(pitch(1.0), 10430);
}

#[test]
fn non_finite_inputs_give_the_stated_zeros() {
    for bad in [f32::NAN, -f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        // A non-finite stick component counts as zero; the other is kept.
        assert_eq!(movement(bad, 0.5), Axis2 { x: 0, y: 16384 }, "{bad}");
        assert_eq!(movement(-0.25, bad), Axis2 { x: -8192, y: 0 }, "{bad}");
        assert_eq!(movement(bad, bad), Axis2::ZERO, "{bad}");
        assert_eq!(movement(bad, 2.0), Axis2 { x: 0, y: AXIS_SCALE }, "{bad}");
        // A non-finite yaw quantises to zero.
        assert_eq!(yaw(bad), 0, "{bad}");
    }
    // A NaN pitch quantises to zero; an infinite one clamps (above).
    assert_eq!(pitch(f32::NAN), 0);
    assert_eq!(pitch(-f32::NAN), 0);
}

#[test]
fn the_edges_are_against_the_previous_call() {
    let mut quantiser = Quantiser::new();
    let mut raw = RawSample::IDLE;
    let mut tick = |held: Actions| {
        raw.begin_frame();
        raw.hold(held);
        let input = quantiser.next(&mut raw);
        input.validate().unwrap();
        (input.held, input.pressed, input.released)
    };
    // A fresh quantiser's previous sample is idle: everything held is pressed.
    assert_eq!(tick(JUMP.union(SPRINT)), (JUMP.union(SPRINT), JUMP.union(SPRINT), Actions::EMPTY));
    assert_eq!(tick(SPRINT), (SPRINT, Actions::EMPTY, JUMP));
    assert_eq!(tick(SPRINT.union(THROW)), (SPRINT.union(THROW), THROW, Actions::EMPTY));
    assert_eq!(tick(THROW), (THROW, Actions::EMPTY, SPRINT));
    assert_eq!(tick(Actions::EMPTY), (Actions::EMPTY, Actions::EMPTY, THROW));
    assert_eq!(tick(Actions::EMPTY), (Actions::EMPTY, Actions::EMPTY, Actions::EMPTY));
    // The game's bits behave as the named ones.
    let game = Actions::EMPTY.with_bit(Actions::GAME_FIRST + 3);
    assert_eq!(tick(game), (game, game, Actions::EMPTY));
}

#[test]
fn a_second_call_on_the_same_raw_sample_carries_no_edges() {
    let mut quantiser = Quantiser::new();
    let mut raw = RawSample { movement_x: 0.25, yaw: 1.0, ..RawSample::IDLE };
    raw.hold(THROW.union(SPRINT));
    let first = quantiser.next(&mut raw);
    assert_eq!(first.pressed, THROW.union(SPRINT));
    // The catch-up ticks of the same frame: the same sample, no edge fires again.
    for _ in 0..3 {
        let again = quantiser.next(&mut raw);
        assert_eq!(again, PlayerInput { pressed: Actions::EMPTY, ..first });
    }
}

// ---- The latch ----

#[test]
fn a_tap_between_polls_is_one_tick_of_held() {
    let mut quantiser = Quantiser::new();
    let mut raw = RawSample::IDLE;
    // Pressed at one poll and released before the next; no tick between them.
    raw.begin_frame();
    raw.hold(JUMP);
    raw.begin_frame();
    assert_eq!(raw.held, Actions::EMPTY);
    let tapped = quantiser.next(&mut raw);
    assert_eq!((tapped.held, tapped.pressed, tapped.released), (JUMP, JUMP, Actions::EMPTY));
    assert_eq!(raw.latched, Actions::EMPTY, "next consumes the latch");
    raw.begin_frame();
    let after = quantiser.next(&mut raw);
    assert_eq!((after.held, after.pressed, after.released), (Actions::EMPTY, Actions::EMPTY, JUMP));
}

#[test]
fn next_leaves_the_latch_empty() {
    let mut raw = RawSample { latched: JUMP.union(THROW), held: SPRINT, ..RawSample::IDLE };
    let input = Quantiser::new().next(&mut raw);
    assert_eq!(input.held, JUMP.union(THROW).union(SPRINT));
    assert_eq!(raw.latched, Actions::EMPTY);
    assert_eq!(raw.held, SPRINT, "next leaves the frame's held set");
}

#[test]
fn an_action_held_across_the_tick_is_not_released_by_the_latch_clearing() {
    let mut quantiser = Quantiser::new();
    let mut raw = RawSample::IDLE;
    raw.begin_frame();
    raw.hold(SPRINT);
    let first = quantiser.next(&mut raw);
    assert_eq!((first.held, first.pressed), (SPRINT, SPRINT));
    // A catch-up tick with no poll: the latch is empty, the frame still holds it.
    let catch_up = quantiser.next(&mut raw);
    assert_eq!(
        (catch_up.held, catch_up.pressed, catch_up.released),
        (SPRINT, Actions::EMPTY, Actions::EMPTY)
    );
    // Further frames that keep holding it.
    for _ in 0..3 {
        raw.begin_frame();
        raw.hold(SPRINT);
        let input = quantiser.next(&mut raw);
        assert_eq!(
            (input.held, input.pressed, input.released),
            (SPRINT, Actions::EMPTY, Actions::EMPTY)
        );
    }
}

// ---- The raw sample ----

#[test]
fn begin_frame_keeps_the_angles_and_the_latch_and_clears_the_rest() {
    let mut raw = RawSample {
        movement_x: 0.5,
        movement_y: -0.25,
        yaw: 1.25,
        pitch: -0.5,
        held: JUMP.union(SPRINT),
        latched: THROW,
    };
    raw.begin_frame();
    assert_eq!(
        raw,
        RawSample {
            movement_x: 0.0,
            movement_y: 0.0,
            yaw: 1.25,
            pitch: -0.5,
            held: Actions::EMPTY,
            latched: THROW,
        }
    );
    assert_eq!(RawSample::default(), RawSample::IDLE);
    assert_eq!(Quantiser::default().next(&mut RawSample::default()), PlayerInput::IDLE);
}

#[test]
fn hold_unions_into_both_sets() {
    let mut raw = RawSample { latched: THROW, ..RawSample::IDLE };
    raw.hold(JUMP);
    raw.hold(JUMP.union(SPRINT));
    raw.hold(Actions::EMPTY);
    assert_eq!(raw.held, JUMP.union(SPRINT));
    assert_eq!(raw.latched, JUMP.union(SPRINT).union(THROW));
}

#[test]
fn offer_movement_keeps_the_larger() {
    let offered = |offers: &[(f32, f32)]| {
        let mut raw = RawSample::IDLE;
        raw.begin_frame();
        for &(x, y) in offers {
            raw.offer_movement(x, y);
        }
        (raw.movement_x.to_bits(), raw.movement_y.to_bits())
    };
    let bits = |x: f32, y: f32| (x.to_bits(), y.to_bits());
    let keys = (0.0, 1.0);
    let stick = (0.3, 0.4);
    let centred = (0.0, 0.0);
    // Whatever the poll order, the larger wins and a centred stick cancels nothing.
    assert_eq!(offered(&[keys, stick]), bits(0.0, 1.0));
    assert_eq!(offered(&[stick, keys]), bits(0.0, 1.0));
    assert_eq!(offered(&[keys, centred]), bits(0.0, 1.0));
    assert_eq!(offered(&[centred, stick]), bits(0.3, 0.4));
    assert_eq!(offered(&[(-0.6, 0.0), (0.0, 0.5)]), bits(-0.6, 0.0));
    // Equal lengths: the first stays.
    assert_eq!(offered(&[(0.6, 0.8), (0.8, -0.6)]), bits(0.6, 0.8));
    // A huge offer compares without overflow.
    assert_eq!(offered(&[(f32::MAX, 0.0), (0.0, -f32::MAX), (1.0, 0.0)]), bits(f32::MAX, 0.0));
    // A non-finite component counts as zero, in the comparison and in what is stored.
    assert_eq!(offered(&[(f32::NAN, 0.2)]), bits(0.0, 0.2));
    assert_eq!(offered(&[(0.5, 0.0), (f32::INFINITY, 0.2)]), bits(0.5, 0.0));
    assert_eq!(offered(&[(f32::NEG_INFINITY, 0.7), (0.5, 0.0)]), bits(0.0, 0.7));
    assert_eq!(offered(&[(f32::NAN, f32::INFINITY)]), bits(0.0, 0.0));
    // A non-finite current movement, assigned directly, counts as zero too.
    let mut raw = RawSample { movement_x: f32::NAN, movement_y: f32::INFINITY, ..RawSample::IDLE };
    raw.offer_movement(0.1, 0.0);
    assert_eq!((raw.movement_x, raw.movement_y), (0.1, 0.0));
}

#[test]
fn turn_wraps_the_yaw_across_many_turns() {
    let steps = if cfg!(miri) { 2_000 } else { 200_000 };
    for delta in [0.37f32, -0.37, 1.9, -3.0, 3.2, 0.000_4] {
        let mut raw = RawSample::IDLE;
        for step in 0..steps {
            let before = raw.yaw;
            raw.turn(delta, 0.0);
            assert!((-PI..PI).contains(&raw.yaw), "{delta} step {step}: {}", raw.yaw);
            // What the same turn would leave unwrapped, quantised.
            let unwrapped = yaw(before + delta);
            let wrapped = yaw(raw.yaw);
            assert!(
                yaw_distance(wrapped, unwrapped) <= 1,
                "{delta} step {step}: {wrapped} against {unwrapped}"
            );
        }
    }
}

#[test]
fn a_small_delta_still_turns_after_many_turns() {
    // Past 8192 rad an accumulated `f32` would swallow this delta; the wrapped yaw does not.
    let mut raw = RawSample::IDLE;
    for _ in 0..3_000 {
        raw.turn(3.0, 0.0);
    }
    let before = raw.yaw;
    raw.turn(4e-4, 0.0);
    let moved = f64::from(raw.yaw) - f64::from(before);
    assert!((moved - 4e-4).abs() < 1e-6, "moved {moved}");
}

#[test]
fn a_zero_turn_never_moves_the_yaw_and_the_wrap_stays_below_pi() {
    for radians in [-PI, -3.0, -1e-30, -0.0, 0.0, 0.1, 3.0, PI.next_down()] {
        let mut raw = RawSample { yaw: radians, ..RawSample::IDLE };
        raw.turn(0.0, 0.0);
        assert_eq!(raw.yaw, radians, "{radians}");
    }
    // A sum just below the true PI rounds up to the `f32` PI, which is above it: a turn
    // back from the `f64` sum lands exactly on the `f32` -PI.
    let mut raw = RawSample { yaw: PI.next_down(), ..RawSample::IDLE };
    raw.turn(1.5e-7, 0.0);
    assert_eq!(raw.yaw, -PI);
    assert_eq!(yaw(raw.yaw), 32768);
    // A yaw assigned past PI is brought into range by the next turn.
    let mut raw = RawSample { yaw: 1e30, ..RawSample::IDLE };
    raw.turn(0.5, 0.0);
    assert!((-PI..PI).contains(&raw.yaw), "{}", raw.yaw);
}

#[test]
fn turn_clamps_the_pitch() {
    let mut raw = RawSample::IDLE;
    raw.turn(0.0, 1.0);
    assert_eq!(raw.pitch, 1.0);
    raw.turn(0.0, 1.0);
    assert_eq!(raw.pitch, FRAC_PI_2);
    raw.turn(0.0, -0.5);
    assert_eq!(raw.pitch, FRAC_PI_2 - 0.5);
    raw.turn(0.0, -10.0);
    assert_eq!(raw.pitch, -FRAC_PI_2);
    raw.turn(0.0, f32::MAX);
    raw.turn(0.0, f32::MAX);
    assert_eq!(raw.pitch, FRAC_PI_2);
}

#[test]
fn a_non_finite_delta_counts_as_zero() {
    let start = RawSample { yaw: 1.0, pitch: 0.5, ..RawSample::IDLE };
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut raw = start;
        raw.turn(bad, 0.25);
        assert_eq!((raw.yaw, raw.pitch), (1.0, 0.75), "{bad}");
        let mut raw = start;
        raw.turn(0.25, bad);
        assert_eq!((raw.yaw, raw.pitch), (1.25, 0.5), "{bad}");
        let mut raw = start;
        raw.turn(bad, bad);
        assert_eq!(raw, start, "{bad}");
    }
}

#[test]
fn a_non_finite_current_angle_counts_as_zero() {
    // The fields are public: the input task or the camera rig may assign anything.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut raw = RawSample { yaw: bad, pitch: bad, ..RawSample::IDLE };
        raw.turn(0.25, -0.5);
        assert_eq!((raw.yaw, raw.pitch), (0.25, -0.5), "{bad}");
        let mut raw = RawSample { yaw: bad, pitch: bad, ..RawSample::IDLE };
        raw.turn(bad, bad);
        assert_eq!((raw.yaw, raw.pitch), (0.0, 0.0), "{bad}");
    }
}

#[test]
fn the_quantised_angles_decode_back_within_half_a_unit() {
    // The input crate decodes; this is the round trip of the two halves.
    let unit = core::f64::consts::TAU / f64::from(UNITS_PER_TURN);
    for k in -400..400 {
        let radians = k as f32 * 0.007_853_98;
        let decoded = Yaw(yaw(radians)).radians().to_f64();
        let wrapped = f64::from(radians).rem_euclid(core::f64::consts::TAU);
        let distance = (decoded - wrapped).abs();
        let distance = distance.min(core::f64::consts::TAU - distance);
        assert!(distance <= unit / 2.0 + 1e-9, "{radians}: {distance}");
        if radians.abs() <= FRAC_PI_2 {
            let decoded = Pitch(pitch(radians)).radians().to_f64();
            assert!((decoded - f64::from(radians)).abs() <= unit / 2.0 + 1e-9, "{radians}");
        }
    }
}
