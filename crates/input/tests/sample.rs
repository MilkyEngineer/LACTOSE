//! The sample as a value (README, "The sample", "The world frame", "The byte image" and
//! "Redundancy and substitution"): the image and its round trip, validation, the decoded
//! angles and directions against the stated formulas, the golden bits, and the
//! substitution rules. Pure functions: nothing here builds a world.

#![forbid(unsafe_code)]

mod common;

use common::*;
use lactose_core::{ErrorKind, Tick};
use lactose_input::{
    AXIS_SCALE, Action, Actions, Axis2, Pitch, PlayerInput, SAMPLE_SIZE, UNITS_PER_TURN, Yaw,
};
use lactose_math::{Quat, Scalar, ScalarBits, Vec2, Vec3};
use lactose_reflect::bytemuck;
use lactose_sim::PlayerSample;

const SWEEP: usize = 20_000;

fn sweep() -> impl Iterator<Item = PlayerInput> {
    let mut rng = Rng::new(0x005a_3b1e);
    (0..SWEEP).map(move |_| valid_sample(&mut rng))
}

fn input(x: i16, y: i16, yaw: u16, pitch: i16) -> PlayerInput {
    PlayerInput {
        movement: Axis2 { x, y },
        yaw: Yaw(yaw),
        pitch: Pitch(pitch),
        ..PlayerInput::IDLE
    }
}

fn with_actions(held: u32, pressed: u32, released: u32) -> PlayerInput {
    PlayerInput {
        held: Actions::from_bits(held),
        pressed: Actions::from_bits(pressed),
        released: Actions::from_bits(released),
        ..PlayerInput::IDLE
    }
}

fn refused(input: &PlayerInput) -> bool {
    let validated = input.validate().map_err(|error| error.kind());
    let decoded = PlayerInput::from_sample_bytes(&input.to_sample_bytes()).map_err(|e| e.kind());
    assert_eq!(validated.is_err(), decoded.is_err(), "{input:?}");
    match validated {
        Ok(()) => false,
        Err(kind) => {
            assert_eq!(kind, ErrorKind::InvalidData, "{input:?}");
            assert_eq!(decoded, Err(ErrorKind::InvalidData), "{input:?}");
            true
        }
    }
}

/// Distance in units in the last place between two finite `Scalar`s of one sign.
fn ulps(a: Scalar, b: Scalar) -> ScalarBits {
    let (a, b) = (a.to_canonical_bits(), b.to_canonical_bits());
    a.max(b) - a.min(b)
}

fn close(a: Vec3, b: Vec3, tolerance: Scalar) -> bool {
    (a - b).length() <= tolerance
}

// ---- The image ----

#[test]
fn the_image_round_trips_and_is_the_memory_image() {
    let mut count = 0;
    for x in sweep().chain([PlayerInput::IDLE]) {
        let bytes = x.to_sample_bytes();
        assert_eq!(bytes.as_slice(), bytemuck::bytes_of(&x));
        assert_eq!(PlayerInput::from_sample_bytes(&bytes).unwrap(), x);
        let decoded: PlayerInput = lactose_reflect::decode(&bytes).unwrap();
        assert_eq!(decoded, x);
        count += 1;
    }
    assert_eq!(count, SWEEP + 1);
}

#[test]
fn the_image_is_each_field_little_endian_at_its_offset() {
    let x = PlayerInput {
        movement: Axis2 { x: 0x0201, y: -0x0403 },
        yaw: Yaw(0x0605),
        pitch: Pitch(-0x0807),
        held: Actions::from_bits(0x0c0b_0a09),
        pressed: Actions::from_bits(0x0c0b_0a09 & 0x0f0e_0d0c),
        released: Actions::from_bits(0x1413_1211 & !0x0c0b_0a09),
    };
    x.validate().unwrap();
    let bytes = x.to_sample_bytes();
    assert_eq!(SAMPLE_SIZE, 20);
    assert_eq!(bytes[0..2], 0x0201i16.to_le_bytes());
    assert_eq!(bytes[2..4], (-0x0403i16).to_le_bytes());
    assert_eq!(bytes[4..6], 0x0605u16.to_le_bytes());
    assert_eq!(bytes[6..8], (-0x0807i16).to_le_bytes());
    assert_eq!(bytes[8..12], x.held.bits().to_le_bytes());
    assert_eq!(bytes[12..16], x.pressed.bits().to_le_bytes());
    assert_eq!(bytes[16..20], x.released.bits().to_le_bytes());
    assert_eq!(PlayerInput::IDLE.to_sample_bytes(), [0; SAMPLE_SIZE]);
}

#[test]
fn an_image_of_any_other_length_is_invalid_data() {
    let image = with_actions(0b11, 0b01, 0).to_sample_bytes();
    let mut long = image.to_vec();
    long.push(0);
    for bytes in [&[][..], &image[..1], &image[..19], &long[..], &[image, image].concat()[..]] {
        let error = PlayerInput::from_sample_bytes(bytes).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{} bytes", bytes.len());
    }
}

#[test]
fn the_frame_entry_carries_the_image_unsubstituted() {
    for x in sweep().take(100) {
        let entry = x.to_player_sample(Tick(77));
        let expected = PlayerSample {
            input_tick: Tick(77),
            substituted: false,
            sample: x.to_sample_bytes().to_vec(),
        };
        assert_eq!(entry, expected);
    }
}

// ---- Validation ----

#[test]
fn each_rule_is_refused_on_its_own() {
    // The pitch.
    assert!(refused(&input(0, 0, 0, Pitch::LIMIT + 1)));
    assert!(refused(&input(0, 0, 0, -Pitch::LIMIT - 1)));
    assert!(refused(&input(0, 0, 0, i16::MAX)));
    assert!(refused(&input(0, 0, 0, i16::MIN)));
    assert!(!refused(&input(0, 0, 0, Pitch::LIMIT)));
    assert!(!refused(&input(0, 0, 0, -Pitch::LIMIT)));
    assert!(!Pitch(Pitch::LIMIT + 1).is_valid() && Pitch(-Pitch::LIMIT).is_valid());
    // `i16::MIN` in either component, which no length test would catch on its own axis.
    assert!(refused(&input(i16::MIN, 0, 0, 0)));
    assert!(refused(&input(0, i16::MIN, 0, 0)));
    // The disc: `(AXIS_SCALE, AXIS_SCALE)` is a diagonal at 1.41; the bound is
    // `AXIS_SCALE² + 2·AXIS_SCALE`, which `(AXIS_SCALE, 255)` meets and `(AXIS_SCALE, 256)`
    // exceeds by two.
    assert!(refused(&input(AXIS_SCALE, AXIS_SCALE, 0, 0)));
    assert!(refused(&input(-AXIS_SCALE, AXIS_SCALE, 0, 0)));
    assert!(refused(&input(AXIS_SCALE, 256, 0, 0)));
    assert!(!refused(&input(AXIS_SCALE, 255, 0, 0)));
    assert!(!refused(&input(-255, -AXIS_SCALE, 0, 0)));
    assert!(!refused(&input(AXIS_SCALE, 0, 0, 0)));
    assert!(!refused(&input(23170, 23170, 0, 0)));
    // An edge names a held action; a release names one not held.
    assert!(refused(&with_actions(0, 0b10, 0)));
    assert!(refused(&with_actions(0b01, 0b11, 0)));
    assert!(refused(&with_actions(0b01, 0, 0b01)));
    assert!(refused(&with_actions(1 << 31, 0, 1 << 31)));
    assert!(!refused(&with_actions(0b11, 0b10, 0b100)));
    // Every other field accepts every bit pattern of its width.
    assert!(!refused(&with_actions(u32::MAX, u32::MAX, 0)));
    assert!(!refused(&with_actions(0, 0, u32::MAX)));
    for yaw in [0, 1, 0x7fff, 0x8000, u16::MAX] {
        assert!(!refused(&input(0, 0, yaw, 0)));
    }
}

#[test]
fn a_sweep_of_valid_samples_is_accepted() {
    assert_eq!(sweep().filter(refused).count(), 0);
}

/// The review's sweep of the quantiser as `lactose-input-host` states it: every direction
/// at lengths about one, beyond one and far beyond (up to `f32::MAX` per component), and
/// the disc's diagonals. All accepted, the worst `x² + y² - AXIS_SCALE²` within the bound's
/// `2·AXIS_SCALE`, and every input at least one long quantised to unit length: within
/// `2·AXIS_SCALE` of `AXIS_SCALE²` either way, never collapsed to zero by an overflow.
#[test]
fn every_output_of_the_quantiser_is_accepted() {
    const DIRECTIONS: u32 = 200_000;
    let scale = i64::from(AXIS_SCALE);
    let mut worst = i64::MIN;
    let mut count = 0u32;
    let mut unit = 0u32;
    let mut check = |movement: Axis2, at_least_one: bool| {
        let (x, y) = (i64::from(movement.x), i64::from(movement.y));
        let excess = x * x + y * y - scale * scale;
        worst = worst.max(excess);
        let sample = PlayerInput { movement, ..PlayerInput::IDLE };
        assert!(!refused(&sample), "the quantiser's {movement:?}");
        if at_least_one {
            assert!(excess.abs() <= 2 * scale, "{movement:?} is not unit length");
            unit += 1;
        }
        count += 1;
    };
    for k in 0..DIRECTIONS {
        let angle = Scalar::from_u32(k) / Scalar::from_u32(DIRECTIONS) * Scalar::TAU;
        let (sin, cos) = angle.sin_cos();
        let (x, y) = (cos.to_f32(), sin.to_f32());
        for length in [1.0f32, 1.5, 0.999_999_9, 1.000_000_1, 1e30] {
            check(quantise_movement(x * length, y * length), length >= 1.0);
        }
    }
    let corner = core::f32::consts::FRAC_1_SQRT_2;
    for (x, y) in [(corner, corner), (-corner, corner), (corner, -corner), (-corner, -corner)] {
        check(quantise_movement(x, y), false);
    }
    for (x, y) in [(f32::MAX, f32::MAX), (-f32::MAX, 3.0), (2e19, -2e19), (0.0, -f32::MAX)] {
        check(quantise_movement(x, y), true);
    }
    for (x, y) in [(f32::NAN, 1.0), (1.0, f32::NAN), (f32::INFINITY, 0.0), (0.0, -1.0)] {
        check(quantise_movement(x, y), false);
    }
    assert_eq!(count, DIRECTIONS * 5 + 12);
    assert_eq!(unit, DIRECTIONS * 4 + 4);
    assert!(worst <= 2 * scale, "worst {worst}");
}

// ---- Decoding ----

#[test]
fn the_angles_decode_to_the_quarter_turns() {
    assert!(same_bits(Yaw(0).radians(), Scalar::ZERO));
    assert!(ulps(Yaw(16384).radians(), Scalar::FRAC_PI_2) <= 1);
    assert!(ulps(Yaw(32768).radians(), Scalar::PI) <= 1);
    assert!(ulps(Yaw(49152).radians(), Scalar::PI + Scalar::FRAC_PI_2) <= 1);
    assert!(Yaw(u16::MAX).radians() < Scalar::TAU);
    assert!(same_bits(Pitch(Pitch::LIMIT).radians(), Scalar::FRAC_PI_2));
    assert!(same_bits(Pitch(-Pitch::LIMIT).radians(), -Scalar::FRAC_PI_2));
    assert!(same_bits(Pitch(0).radians(), Scalar::ZERO));
    let unit = Axis2 { x: AXIS_SCALE, y: 0 }.to_vec2();
    assert!(same_bits(unit.x, Scalar::ONE) && same_bits(unit.y, Scalar::ZERO));
    let back = Axis2 { x: 0, y: -AXIS_SCALE }.to_vec2();
    assert!(same_bits(back.x, Scalar::ZERO) && same_bits(back.y, Scalar::NEG_ONE));
}

/// Every derived value is the README's formula, written out here, bit for bit.
#[test]
fn the_derived_values_are_the_stated_formulas() {
    let turn = Scalar::from_u32(UNITS_PER_TURN);
    let scale = Scalar::from_i32(i32::from(AXIS_SCALE));
    for x in sweep() {
        let yaw = Scalar::from_u32(u32::from(x.yaw.0)) / turn * Scalar::TAU;
        let pitch = Scalar::from_i32(i32::from(x.pitch.0)) / turn * Scalar::TAU;
        let m = Vec2::new(
            Scalar::from_i32(i32::from(x.movement.x)) / scale,
            Scalar::from_i32(i32::from(x.movement.y)) / scale,
        );
        let (sin_y, cos_y) = yaw.sin_cos();
        let (sin_p, cos_p) = pitch.sin_cos();
        let aim = Vec3::new(-(cos_p * sin_y), sin_p, -(cos_p * cos_y));
        let walk = Vec3::new(m.x * cos_y - m.y * sin_y, Scalar::ZERO, -(m.x * sin_y + m.y * cos_y));
        assert!(same_bits(x.yaw.radians(), yaw), "{x:?}");
        assert!(same_bits(x.pitch.radians(), pitch), "{x:?}");
        let decoded = x.movement.to_vec2();
        assert!(same_bits(decoded.x, m.x) && same_bits(decoded.y, m.y), "{x:?}");
        assert!(same_quat(x.yaw.rotation(), Quat::from_rotation_y(yaw)), "{x:?}");
        assert!(same_vec3(x.aim(), aim), "{x:?}");
        assert!(same_vec3(x.world_movement(), walk), "{x:?}");
        assert!(x.yaw.radians() >= Scalar::ZERO && x.yaw.radians() < Scalar::TAU);
        assert!(x.pitch.radians().abs() <= Scalar::FRAC_PI_2);
    }
}

// ---- Directions ----

#[test]
fn the_cardinal_directions() {
    let neg_z = -Vec3::Z;
    assert_eq!(PlayerInput::IDLE.aim(), neg_z);
    let eps = Scalar::EPSILON;
    assert!(close(input(0, 0, 16384, 0).aim(), -Vec3::X, eps));
    assert!(close(input(0, 0, 49152, 0).aim(), Vec3::X, eps));
    assert!(close(input(0, 0, 32768, 0).aim(), Vec3::Z, eps));
    assert!(close(input(0, 0, 0, Pitch::LIMIT).aim(), Vec3::Y, eps));
    assert!(close(input(0, 0, 12345, -Pitch::LIMIT).aim(), -Vec3::Y, eps));
    // Movement at yaw zero: forward is `-Z`, right is `+X`.
    assert_eq!(input(0, AXIS_SCALE, 0, 0).world_movement(), neg_z);
    assert_eq!(input(AXIS_SCALE, 0, 0, 0).world_movement(), Vec3::X);
    assert_eq!(input(0, 0, 0, 0).world_movement(), Vec3::ZERO);
    // A quarter turn left: forward is `-X`, right is `-Z`; pitch never moves the player.
    assert!(close(input(0, AXIS_SCALE, 16384, Pitch::LIMIT).world_movement(), -Vec3::X, eps));
    assert!(close(input(AXIS_SCALE, 0, 16384, 0).world_movement(), neg_z, eps));
}

#[test]
fn the_rotation_turns_minus_z_onto_the_horizontal_aim() {
    let tolerance = Scalar::from_f64(1e-15);
    for units in (0..=u32::from(u16::MAX)).step_by(97) {
        let level = input(0, 0, units as u16, 0);
        let turned = level.yaw.rotation() * -Vec3::Z;
        assert!(close(turned, level.aim(), tolerance), "yaw {units}");
    }
}

#[test]
fn the_directions_have_unit_length_up_to_rounding() {
    let rounding = Scalar::from_f64(1e-15);
    // A valid vector exceeds the unit disc by rounding only: `sqrt(1 + 2 / AXIS_SCALE)`.
    let over = Scalar::ONE + Scalar::ONE / Scalar::from_i32(i32::from(AXIS_SCALE)) + rounding;
    for x in sweep() {
        assert!((x.aim().length() - Scalar::ONE).abs() <= rounding, "{x:?}");
        assert!(x.world_movement().length() <= over, "{x:?}");
    }
}

/// The review's probe of the world frame against `Quat`: `aim` is the yaw about `+Y`
/// composed with the pitch about `+X` applied to `-Z`, and `world_movement` takes `m.y`
/// along the yawed forward and `m.x` along the yawed right.
#[test]
fn the_world_frame_is_the_quaternion_frame() {
    let tolerance = Scalar::from_f64(1e-12);
    let mut rng = Rng::new(0xf4a3e);
    let mut count = 0;
    for yaw in (0..=u32::from(u16::MAX)).step_by(257) {
        for pitch in (-i32::from(Pitch::LIMIT)..=i32::from(Pitch::LIMIT)).step_by(129) {
            let x = PlayerInput {
                movement: valid_movement(&mut rng),
                ..input(0, 0, yaw as u16, pitch as i16)
            };
            let facing = Quat::from_rotation_y(x.yaw.radians());
            let looking = facing * Quat::from_rotation_x(x.pitch.radians());
            assert!(close(x.aim(), looking * -Vec3::Z, tolerance), "{x:?}");
            let m = x.movement.to_vec2();
            let walk = facing * -Vec3::Z * m.y + facing * Vec3::X * m.x;
            assert!(close(x.world_movement(), walk, tolerance), "{x:?}");
            count += 1;
        }
    }
    assert_eq!(count, 256 * 255);
}

/// The golden bits: a small table of samples and the canonical bits of every value derived
/// from them, pinned. This file runs on every target CI tests (aarch64 here, linux-x64 in
/// CI), and `tests/sour.rs` hashes the same values under SOUR's architecture mode.
#[test]
fn the_golden_bits() {
    let table = [
        input(0, 0, 0, 0),
        input(AXIS_SCALE, 0, 16384, Pitch::LIMIT),
        input(-12345, 23456, 1, -1),
        input(23170, -23170, 32769, 9001),
        input(-1, AXIS_SCALE, 51234, -16000),
        input(7, -32000, u16::MAX, 4321),
    ];
    let bits = |x: &PlayerInput| -> [ScalarBits; 12] {
        let (aim, walk, rotation) = (x.aim(), x.world_movement(), x.yaw.rotation());
        [
            x.yaw.radians(),
            x.pitch.radians(),
            aim.x,
            aim.y,
            aim.z,
            walk.x,
            walk.y,
            walk.z,
            rotation.x,
            rotation.y,
            rotation.z,
            rotation.w,
        ]
        .map(Scalar::to_canonical_bits)
    };
    let actual: Vec<[ScalarBits; 12]> = table.iter().map(bits).collect();
    let printed: String = actual
        .iter()
        .map(|row| {
            let row: Vec<String> = row.iter().map(|bits| format!("{bits:#018x}")).collect();
            format!("    [{}],\n", row.join(", "))
        })
        .collect();
    assert_eq!(actual.as_slice(), GOLDEN, "the table on this target:\n[\n{printed}]");
}

/// Generated on aarch64-apple-darwin; the row order is the table's, the column order the
/// closure's. A mismatch prints the table of the target it ran on.
#[rustfmt::skip]
const GOLDEN: &[[ScalarBits; 12]] = &[
    [0x0000000000000000, 0x0000000000000000, 0x8000000000000000, 0x0000000000000000, 0xbff0000000000000, 0x0000000000000000, 0x0000000000000000, 0x8000000000000000, 0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0x3ff0000000000000],
    [0x3ff921fb54442d18, 0x3ff921fb54442d18, 0xbc91a62633145c07, 0x3ff0000000000000, 0xb93377ce858a5d48, 0x3c91a62633145c07, 0x0000000000000000, 0xbff0000000000000, 0x0000000000000000, 0x3fe6a09e667f3bcc, 0x0000000000000000, 0x3fe6a09e667f3bcd],
    [0x3f1921fb54442d18, 0xbf1921fb54442d18, 0xbf1921fb51aeb57c, 0xbf1921fb539ecf31, 0xbfeffffffb10b0ce, 0xbfd81dd012f6af60, 0x0000000000000000, 0xbfe6e7e20e84ec1b, 0x0000000000000000, 0x3f0921fb541ad59e, 0x0000000000000000, 0x3fefffffff62161a],
    [0x4009222d983ad5a0, 0x3feb9d5e6e2e88ac, 0x3f105751bb695bc7, 0x3fe8500a387bb15d, 0x3fe4ce5e14910f6b, 0xbfe6a13b6c120529, 0x0000000000000000, 0xbfe6a01f1325b28b, 0x0000000000000000, 0x3fefffffff62161a, 0x0000000000000000, 0xbf0921fb541a853a],
    [0x4013a5e2dd367449, 0xbff88b2f704a9409, 0x3fa2789a8f9c9b3f, 0xbfeffa72effef75d, 0xbf7de4e54ba9f4d4, 0x3fef5d4a89c81c5f, 0x0000000000000000, 0xbfc9626d6ce8cf52, 0x0000000000000000, 0x3fe4429f302e908d, 0x0000000000000000, 0xbfe8c4f6a764e9e5],
    [0x401921e23248d8d4, 0x3fda836932952bd2, 0x3f1701bb68300caa, 0x3fd9c2ed61c0ff97, 0xbfed4b0a2c853f59, 0x3f1f750f762457c1, 0x0000000000000000, 0x3fef403e8913166f, 0x0000000000000000, 0x3f0921fb541aef1f, 0x0000000000000000, 0xbfefffffff62161a],
];

// ---- Substitution ----

#[test]
fn a_substitute_keeps_the_hold_and_drops_the_edges() {
    for x in sweep() {
        let substitute = x.substitute();
        assert_eq!(substitute.movement, x.movement);
        assert_eq!((substitute.yaw, substitute.pitch, substitute.held), (x.yaw, x.pitch, x.held));
        assert_eq!((substitute.pressed, substitute.released), (Actions::EMPTY, Actions::EMPTY));
        substitute.validate().unwrap();
        assert_eq!(substitute.substitute(), substitute);
    }
    assert_eq!(PlayerInput::IDLE.substitute(), PlayerInput::IDLE);
}

#[test]
fn the_repair_restores_the_swallowed_edges() {
    let throw = Actions::EMPTY.with(Action::Throw);
    let sprint = Actions::EMPTY.with(Action::Sprint);
    // A press whose first sample was lost: the client's next sample holds `Throw` with no
    // edge of its own; the substitute did not hold it.
    let delivered = PlayerInput { held: throw, ..input(100, 200, 300, 400) };
    let repaired = delivered.after_substitute(&PlayerInput::IDLE.substitute());
    assert_eq!(repaired.pressed, throw);
    assert_eq!(repaired.released, Actions::EMPTY);
    repaired.validate().unwrap();
    assert_eq!(PlayerInput { pressed: Actions::EMPTY, ..repaired }, delivered);
    // A release swallowed: the substitute held `Sprint`, the delivered sample does not.
    let applied = PlayerInput { held: sprint, ..PlayerInput::IDLE };
    let repaired = PlayerInput::IDLE.after_substitute(&applied);
    assert_eq!((repaired.pressed, repaired.released), (Actions::EMPTY, sprint));
    repaired.validate().unwrap();
    // The delivered sample's own release of an action the substitute never held is
    // dropped: no applied sample held it, so nothing is released.
    let delivered =
        PlayerInput { released: Actions::EMPTY.with(Action::Interact), ..PlayerInput::IDLE };
    let repaired = delivered.after_substitute(&PlayerInput::IDLE);
    assert_eq!(repaired.released, Actions::EMPTY);
    // Its own press of an action the substitute already held is dropped: it was held.
    let jump = Actions::EMPTY.with(Action::Jump);
    let delivered = PlayerInput { held: jump, pressed: jump, ..PlayerInput::IDLE };
    let repaired = delivered.after_substitute(&PlayerInput { held: jump, ..PlayerInput::IDLE });
    assert_eq!(repaired.pressed, Actions::EMPTY);
}

/// The review's exhaustive check: over every valid (sample, applied) pair of a four-action
/// set (three named, one of the game's top bit), the repaired edges are exactly the
/// differences of the two held sets, everything else is the sample's, and the result is
/// valid.
#[test]
fn the_repair_over_every_pair_of_a_small_action_set() {
    let bits: [u32; 4] =
        [1 << Action::Jump.bit(), 1 << Action::Throw.bit(), 1 << Action::Sprint.bit(), 1 << 31];
    let set = |index: u32| -> Actions {
        Actions::from_bits(
            bits.iter().enumerate().filter(|&(i, _)| index & (1 << i) != 0).map(|(_, b)| b).sum(),
        )
    };
    let mut pairs = 0u32;
    for (h, p, r) in
        (0..16).flat_map(|h| (0..16).flat_map(move |p| (0..16).map(move |r| (h, p, r))))
    {
        let sample =
            PlayerInput { held: set(h), pressed: set(p), released: set(r), ..input(1, 2, 3, 4) };
        if sample.validate().is_err() {
            continue;
        }
        for (ah, ap, ar) in
            (0..16).flat_map(|h| (0..16).flat_map(move |p| (0..16).map(move |r| (h, p, r))))
        {
            let applied = PlayerInput {
                held: set(ah),
                pressed: set(ap),
                released: set(ar),
                ..input(5, 6, 7, 8)
            };
            if applied.validate().is_err() {
                continue;
            }
            let repaired = sample.after_substitute(&applied);
            assert_eq!(repaired.pressed, sample.held.without(applied.held));
            assert_eq!(repaired.released, applied.held.without(sample.held));
            assert_eq!(
                PlayerInput { pressed: sample.pressed, released: sample.released, ..repaired },
                sample
            );
            repaired.validate().unwrap();
            pairs += 1;
        }
    }
    // 4^4 valid (held, pressed, released) triples per side: each action held and maybe
    // pressed, or not held and maybe released.
    assert_eq!(pairs, 256 * 256);
}

/// `net`'s procedure end to end over a held sequence with losses: substitute every lost
/// tick, repair the first delivered sample after a run of substitutes. Every applied
/// sample's edges then match the applied held sequence exactly, and every applied sample is
/// valid.
#[test]
fn applied_edges_follow_the_applied_holds_through_losses() {
    let mut rng = Rng::new(0x105e5);
    let mut client_held = Actions::EMPTY;
    let mut applied = PlayerInput::IDLE;
    let mut substituting = false;
    let mut presses = 0;
    for tick in 0..20_000u32 {
        let held = Actions::from_bits(rng.next_u32() & 0b1_0111);
        let sample = PlayerInput {
            held,
            pressed: held.without(client_held),
            released: client_held.without(held),
            ..PlayerInput::IDLE
        };
        client_held = held;
        let lost = rng.next_u32().is_multiple_of(4);
        let next = if lost {
            substituting = true;
            applied.substitute()
        } else if substituting {
            substituting = false;
            sample.after_substitute(&applied)
        } else {
            sample
        };
        next.validate().unwrap();
        assert_eq!(next.pressed, next.held.without(applied.held), "tick {tick}");
        assert_eq!(next.released, applied.held.without(next.held), "tick {tick}");
        presses += next.pressed.bits().count_ones();
        applied = next;
    }
    assert!(presses > 1000);
}
