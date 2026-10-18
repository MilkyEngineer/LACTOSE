//! The quantised wire values (README, "Quantisation"): the position round trip over a
//! sweep, saturation, NaN, ties; the rotation's sign rule, the measured rate at which one
//! trip moves a uniform rotation and the worst cases found; the zero, NaN and infinite
//! quaternions; and the golden bits of `dequantise`, pinned so that every target that
//! runs this file (aarch64 here, x86-64 in CI) must produce them. `sour.rs` records the
//! same conversions under SOUR, whose architecture mode compares two machines.

#![forbid(unsafe_code)]

use lactose_math::{Quat, Scalar, Vec3};
use lactose_net_host::quant::{POSITION_UNITS_PER_METRE, ROTATION_SCALE};
use lactose_net_host::{QuantisedPosition, QuantisedRotation};

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A uniform scalar in `[-1, 1)` from the generator's top 53 bits.
fn unit(state: &mut u64) -> Scalar {
    let top = i64::try_from(xorshift(state) >> 11).expect("53 bits");
    Scalar::from_i64(top) / Scalar::from_i64(1 << 52) - Scalar::ONE
}

fn metres(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(Scalar::from_f64(x), Scalar::from_f64(y), Scalar::from_f64(z))
}

fn position(x: i32, y: i32, z: i32) -> QuantisedPosition {
    QuantisedPosition { x, y, z }
}

fn rotation(x: i16, y: i16, z: i16, w: i16) -> QuantisedRotation {
    QuantisedRotation { x, y, z, w }
}

const NOT_A_ROTATION: QuantisedRotation = QuantisedRotation { x: 0, y: 0, z: 0, w: 0 };

#[test]
fn a_position_round_trips_for_every_integer_of_a_sweep() {
    let mut values: Vec<i32> = (-70_000..=70_000).collect();
    for shift in 0..31 {
        let power = 1_i32 << shift;
        values.extend([power - 1, power, power + 1, -power - 1, -power, -power + 1]);
    }
    values.extend([i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX, 1 << 24, (1 << 24) + 1]);
    let mut state = 0x2545_F491_4F6C_DD1D;
    values.extend((0..200_000).map(|_| (xorshift(&mut state) >> 32) as u32 as i32));
    for q in values {
        let quantised = position(q, q.wrapping_neg(), q / 3);
        assert_eq!(QuantisedPosition::quantise(quantised.dequantise()), quantised, "{q}");
    }
}

#[test]
fn a_position_rounds_to_nearest_with_ties_away_from_zero() {
    let unit = 1.0 / f64::from(POSITION_UNITS_PER_METRE);
    assert_eq!(QuantisedPosition::quantise(metres(1.0, -1.0, 0.0)), position(1024, -1024, 0));
    assert_eq!(
        QuantisedPosition::quantise(metres(0.5 * unit, -0.5 * unit, 1.5 * unit)),
        position(1, -1, 2)
    );
    assert_eq!(
        QuantisedPosition::quantise(metres(0.49 * unit, -0.51 * unit, 2.5 * unit)),
        position(0, -1, 3)
    );
    assert_eq!(QuantisedPosition::quantise(Vec3::ZERO), QuantisedPosition::ZERO);
    assert_eq!(QuantisedPosition::ZERO.dequantise(), Vec3::ZERO);
}

#[test]
fn a_position_saturates_at_the_i32_limits_and_maps_nan_to_zero() {
    let far = Scalar::from_f64(1.0e300);
    let edge = f64::from(i32::MAX) / f64::from(POSITION_UNITS_PER_METRE);
    let cases = [
        (Vec3::new(far, -far, Scalar::NAN), position(i32::MAX, i32::MIN, 0)),
        (
            Vec3::new(Scalar::INFINITY, Scalar::NEG_INFINITY, Scalar::ZERO),
            position(i32::MAX, i32::MIN, 0),
        ),
        (metres(edge + 1.0, -edge - 1.0, edge), position(i32::MAX, i32::MIN, i32::MAX)),
    ];
    for (relative, expected) in cases {
        assert_eq!(QuantisedPosition::quantise(relative), expected);
    }
}

#[test]
fn a_rotation_and_its_negation_quantise_alike() {
    let mut state = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..100_000 {
        let q = Quat::new(unit(&mut state), unit(&mut state), unit(&mut state), unit(&mut state));
        assert_eq!(QuantisedRotation::quantise(q), QuantisedRotation::quantise(-q), "{q:?}");
    }
    // The sign is decided on the integers: `w` rounds to zero here, so `x` decides.
    let tiny_w = Quat::new(Scalar::NEG_ONE, Scalar::ZERO, Scalar::ZERO, Scalar::from_f64(1.0e-6));
    let quantised = QuantisedRotation::quantise(tiny_w);
    assert_eq!(quantised, rotation(ROTATION_SCALE, 0, 0, 0));
    assert_eq!(QuantisedRotation::quantise(-tiny_w), quantised);
    assert_eq!(QuantisedRotation::quantise(quantised.dequantise()), quantised);
}

/// A uniformly distributed rotation: a point uniform in the unit 4-ball, by rejection
/// from the cube, normalised.
fn uniform_rotation(state: &mut u64) -> Quat {
    loop {
        let q = Quat::new(unit(state), unit(state), unit(state), unit(state));
        let length_squared = q.length_squared();
        if length_squared <= Scalar::ONE && length_squared > Scalar::from_f64(1e-6) {
            return q.normalize();
        }
    }
}

/// How one trip through `dequantise` and `quantise` moved a value: the largest step of a
/// component and how many components moved.
fn trip(first: QuantisedRotation) -> (QuantisedRotation, u16, usize) {
    let second = QuantisedRotation::quantise(first.dequantise());
    let steps = [
        second.x.abs_diff(first.x),
        second.y.abs_diff(first.y),
        second.z.abs_diff(first.z),
        second.w.abs_diff(first.w),
    ];
    let largest = steps.iter().copied().max().expect("four");
    (second, largest, steps.iter().filter(|&&step| step != 0).count())
}

/// The README's measured claim, over uniform rotations: one trip moves about one value in
/// seventy, by one unit in each component that moves, and a value that made the trip once
/// almost always stays.
#[test]
fn a_uniform_rotation_moves_by_one_unit_in_one_trip_at_a_measured_rate() {
    const DRAWS: u32 = 400_000;
    let mut state = 0x9E37_79B9_7F4A_7C15;
    let mut moved_in = [0_u32; 5];
    let mut unstable = 0_u32;
    for _ in 0..DRAWS {
        let first = QuantisedRotation::quantise(uniform_rotation(&mut state));
        let (second, largest, moved) = trip(first);
        assert!(largest <= 1, "{first:?} moved by {largest}");
        moved_in[moved] += 1;
        let (third, _, _) = trip(second);
        unstable += u32::from(third != second);
    }
    let moved: u32 = moved_in[1..].iter().sum();
    println!("{moved} of {DRAWS} moved, by components {moved_in:?}, {unstable} unstable");
    let per_ten_thousand = u64::from(moved) * 10_000 / u64::from(DRAWS);
    assert!((120..=190).contains(&per_ten_thousand), "{moved} of {DRAWS} moved");
    assert_eq!(moved_in[4], 0);
    assert!(moved_in[3] * 1_000 <= DRAWS, "{moved_in:?}");
    assert!(unstable * 10_000 <= DRAWS, "{unstable} unstable");
}

/// The worst cases found near half-unit boundaries (review r1, M2): a value one trip moves
/// in three components, and one that the second trip moves again before it settles.
#[test]
fn a_rotation_may_move_in_three_components_and_settle_only_after_two_trips() {
    let first = rotation(-19250, -18758, -18706, 1131);
    let (second, largest, moved) = trip(first);
    assert_eq!((second, largest, moved), (rotation(-19251, -18759, -18707, 1131), 1, 3));
    let (third, _, _) = trip(second);
    assert_eq!(third, rotation(-19250, -18759, -18707, 1131));
    assert_eq!(trip(third).0, third);
}

#[test]
fn a_rotation_saturates_at_the_scale() {
    let axes = [
        (Quat::IDENTITY, QuantisedRotation::IDENTITY),
        (
            Quat::new(Scalar::ONE, Scalar::ZERO, Scalar::ZERO, Scalar::ZERO),
            rotation(32_767, 0, 0, 0),
        ),
        (
            Quat::new(Scalar::ZERO, Scalar::NEG_ONE, Scalar::ZERO, Scalar::ZERO),
            rotation(0, 32_767, 0, 0),
        ),
        (
            Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, Scalar::NEG_ONE),
            QuantisedRotation::IDENTITY,
        ),
        // Not unit length: normalised first.
        (
            Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::from_i32(-7), Scalar::ZERO),
            rotation(0, 0, 32_767, 0),
        ),
    ];
    for (q, expected) in axes {
        assert_eq!(QuantisedRotation::quantise(q), expected, "{q:?}");
    }
    assert_eq!(QuantisedRotation::default(), QuantisedRotation::IDENTITY);
    assert_eq!(QuantisedRotation::IDENTITY.dequantise(), Quat::IDENTITY);
}

#[test]
fn no_rotation_quantises_to_all_zero_and_back_to_the_identity() {
    let nan = Scalar::NAN;
    let none = [
        Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, Scalar::ZERO),
        Quat::new(nan, Scalar::ZERO, Scalar::ZERO, Scalar::ONE),
        Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, nan),
        Quat::new(Scalar::INFINITY, Scalar::ZERO, Scalar::ZERO, Scalar::ONE),
        Quat::new(Scalar::ONE, Scalar::ZERO, Scalar::NEG_INFINITY, Scalar::ONE),
    ];
    for q in none {
        assert_eq!(QuantisedRotation::quantise(q), NOT_A_ROTATION, "{q:?}");
    }
    assert_eq!(NOT_A_ROTATION.dequantise(), Quat::IDENTITY);
    // Every finite non-zero quaternion is a rotation, however large or small.
    let half = Scalar::HALF;
    for scale in [1.0e-300, 1.0e-160, 1.0e160, 1.0e300, f64::MIN_POSITIVE, f64::MAX] {
        let s = Scalar::from_f64(scale);
        let q = Quat::new(half * s, -half * s, half * s, half * s);
        let expected = QuantisedRotation::quantise(Quat::new(half, -half, half, half));
        assert_eq!(QuantisedRotation::quantise(q), expected, "{scale}");
        assert_ne!(expected, NOT_A_ROTATION);
    }
}

#[test]
fn quantising_a_known_rotation_gives_its_integers() {
    let quarter_turn = Quat::from_rotation_z(Scalar::FRAC_PI_2);
    assert_eq!(QuantisedRotation::quantise(quarter_turn), rotation(0, 0, 23_170, 23_170));
    let turn = Quat::from_rotation_x(Scalar::from_f64(1.0));
    assert_eq!(QuantisedRotation::quantise(turn), rotation(15_709, 0, 0, 28_756));
}

/// The bits of `dequantise` for a small table, pinned. The position conversion is an
/// exact division by a power of two; the rotation conversion divides and normalises, and
/// its bits are whatever `lactose_math` gives on every target.
#[test]
fn dequantise_gives_the_golden_bits() {
    let positions: [(QuantisedPosition, [u64; 3]); 3] = [
        (position(1, -1536, 0), [0x3F50_0000_0000_0000, 0xBFF8_0000_0000_0000, 0]),
        (
            position(i32::MAX, i32::MIN, 1_000_000),
            [0x413F_FFFF_FFC0_0000, 0xC140_0000_0000_0000, 0x408E_8480_0000_0000],
        ),
        (
            position(-3, 7, 123_456_789),
            [0xBF68_0000_0000_0000, 0x3F7C_0000_0000_0000, 0x40FD_6F34_5400_0000],
        ),
    ];
    for (quantised, bits) in positions {
        let value = quantised.dequantise();
        let got = [value.x, value.y, value.z].map(Scalar::to_canonical_bits);
        assert_eq!(got, bits, "{quantised:?}: {got:#018X?}");
    }
    // Cross-checked against IEEE double arithmetic outside Rust: each component over
    // 32767, then divided by the square root of ((x*x + y*y) + z*z) + w*w.
    let rotations: [(QuantisedRotation, [u64; 4]); 4] = [
        (QuantisedRotation::IDENTITY, [0, 0, 0, 0x3FF0_0000_0000_0000]),
        (rotation(16_384, 16_384, 16_384, 16_384), [0x3FE0_0000_0000_0000; 4]),
        (
            rotation(12_345, -2_345, 345, 30_000),
            [
                0x3FD8_4A33_3B72_69D8,
                0xBFB2_74B9_54E1_753B,
                0x3F85_B8DE_7F31_C004,
                0x3FED_8387_5957_12CD,
            ],
        ),
        (
            rotation(-1, 1, -32_767, 2),
            [
                0xBF00_001F_FF7F_FC00,
                0x3F00_001F_FF7F_FC00,
                0xBFEF_FFFF_FE7F_FA00,
                0x3F10_001F_FF7F_FC00,
            ],
        ),
    ];
    for (quantised, bits) in rotations {
        let value = quantised.dequantise();
        let got = value.to_array().map(Scalar::to_canonical_bits);
        assert_eq!(got, bits, "{quantised:?}: {got:#018X?}");
    }
}
