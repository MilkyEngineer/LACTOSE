//! The documented `min`, `max` and `clamp` rules, on every operand position.

mod common;

use common::Rng;
use lactose_math::{Scalar, Vec3};

const NAN_BITS: u64 = 0x7ff8_0000_0000_0000;
const POSITIVE_ZERO: u64 = 0;
const NEGATIVE_ZERO: u64 = 1 << 63;

fn bits(value: Scalar) -> u64 {
    value.to_canonical_bits()
}

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

/// A NaN with the sign set and a payload, as arithmetic on x86-64 produces.
fn odd_nan() -> Scalar {
    s(f64::from_bits(0xfff8_0000_dead_beef))
}

/// The documented order as an integer key: numeric order refined by `-0 < +0`. `None` for
/// NaN.
fn key(value: Scalar) -> Option<i128> {
    if value.is_nan() {
        return None;
    }
    let bits = bits(value);
    let magnitude = i128::from(bits & !(1 << 63));
    // Negative values sort below every positive one, and `-0` (magnitude zero) lands just
    // below `+0`.
    Some(if bits >> 63 == 1 { -magnitude - 1 } else { magnitude })
}

fn reference_min(a: Scalar, b: Scalar) -> u64 {
    match (key(a), key(b)) {
        (Some(ka), Some(kb)) => bits(if ka <= kb { a } else { b }),
        _ => NAN_BITS,
    }
}

fn reference_max(a: Scalar, b: Scalar) -> u64 {
    match (key(a), key(b)) {
        (Some(ka), Some(kb)) => bits(if ka >= kb { a } else { b }),
        _ => NAN_BITS,
    }
}

fn interesting() -> Vec<Scalar> {
    let mut values = vec![
        Scalar::NAN,
        odd_nan(),
        Scalar::NEG_INFINITY,
        Scalar::MIN,
        s(-1.0),
        -Scalar::MIN_POSITIVE,
        Scalar::from_canonical_bits(0x8000_0000_0000_0001),
        -Scalar::ZERO,
        Scalar::ZERO,
        Scalar::from_canonical_bits(1),
        Scalar::MIN_POSITIVE,
        s(1.0),
        Scalar::MAX,
        Scalar::INFINITY,
    ];
    let mut rng = Rng(0x005e_1ec7);
    values.extend((0..18).map(|_| rng.any()));
    values
}

#[test]
fn signed_zeros_in_every_position() {
    let (pz, nz) = (Scalar::ZERO, -Scalar::ZERO);
    assert_eq!(bits(pz.min(nz)), NEGATIVE_ZERO);
    assert_eq!(bits(nz.min(pz)), NEGATIVE_ZERO);
    assert_eq!(bits(pz.min(pz)), POSITIVE_ZERO);
    assert_eq!(bits(nz.min(nz)), NEGATIVE_ZERO);

    assert_eq!(bits(pz.max(nz)), POSITIVE_ZERO);
    assert_eq!(bits(nz.max(pz)), POSITIVE_ZERO);
    assert_eq!(bits(pz.max(pz)), POSITIVE_ZERO);
    assert_eq!(bits(nz.max(nz)), NEGATIVE_ZERO);

    // clamp(x, lo, hi) over all eight sign combinations of three zeros: the result is `x`
    // raised to at least `lo`, then lowered to at most `hi`, under -0 < +0.
    for (x, lo, hi, expected) in [
        (pz, pz, pz, POSITIVE_ZERO),
        (pz, pz, nz, NEGATIVE_ZERO),
        (pz, nz, pz, POSITIVE_ZERO),
        (pz, nz, nz, NEGATIVE_ZERO),
        (nz, pz, pz, POSITIVE_ZERO),
        (nz, pz, nz, NEGATIVE_ZERO),
        (nz, nz, pz, NEGATIVE_ZERO),
        (nz, nz, nz, NEGATIVE_ZERO),
    ] {
        assert_eq!(bits(x.clamp(lo, hi)), expected, "clamp({x:?}, {lo:?}, {hi:?})");
    }
    assert_eq!(bits(nz.clamp(pz, s(1.0))), POSITIVE_ZERO);
    assert_eq!(bits(pz.clamp(s(-1.0), nz)), NEGATIVE_ZERO);
}

#[test]
fn nan_in_every_position() {
    let one = s(1.0);
    for nan in [Scalar::NAN, odd_nan()] {
        assert_eq!(bits(nan.min(one)), NAN_BITS);
        assert_eq!(bits(one.min(nan)), NAN_BITS);
        assert_eq!(bits(nan.min(nan)), NAN_BITS);
        assert_eq!(bits(nan.max(one)), NAN_BITS);
        assert_eq!(bits(one.max(nan)), NAN_BITS);
        assert_eq!(bits(nan.max(nan)), NAN_BITS);
        assert_eq!(bits(nan.min(Scalar::NEG_INFINITY)), NAN_BITS);
        assert_eq!(bits(Scalar::INFINITY.max(nan)), NAN_BITS);

        assert_eq!(bits(nan.clamp(s(0.0), one)), NAN_BITS);
        assert_eq!(bits(s(0.5).clamp(nan, one)), NAN_BITS);
        assert_eq!(bits(s(0.5).clamp(s(0.0), nan)), NAN_BITS);
        // Even where the NaN bound could not have been the one that binds.
        assert_eq!(bits(s(5.0).clamp(nan, one)), NAN_BITS);
        assert_eq!(bits(s(-5.0).clamp(s(0.0), nan)), NAN_BITS);
    }
}

#[test]
fn clamp_ordinary_and_inverted_bounds() {
    assert_eq!(s(0.5).clamp(s(0.0), s(1.0)), s(0.5));
    assert_eq!(s(-3.0).clamp(s(0.0), s(1.0)), s(0.0));
    assert_eq!(s(3.0).clamp(s(0.0), s(1.0)), s(1.0));
    assert_eq!(Scalar::INFINITY.clamp(s(0.0), s(1.0)), s(1.0));
    assert_eq!(Scalar::NEG_INFINITY.clamp(s(0.0), s(1.0)), s(0.0));
    // lo > hi never panics and gives hi.
    for x in [s(-9.0), s(0.5), s(9.0)] {
        assert_eq!(x.clamp(s(1.0), s(0.0)), s(0.0));
    }
}

#[test]
fn min_max_follow_the_documented_order() {
    let values = interesting();
    for &a in &values {
        for &b in &values {
            assert_eq!(bits(a.min(b)), reference_min(a, b), "min({a:?}, {b:?})");
            assert_eq!(bits(a.max(b)), reference_max(a, b), "max({a:?}, {b:?})");
        }
    }
}

#[test]
fn min_max_are_commutative_and_associative() {
    let values = interesting();
    for &a in &values {
        for &b in &values {
            assert_eq!(bits(a.min(b)), bits(b.min(a)), "min({a:?}, {b:?})");
            assert_eq!(bits(a.max(b)), bits(b.max(a)), "max({a:?}, {b:?})");
            for &c in &values {
                assert_eq!(bits(a.min(b).min(c)), bits(a.min(b.min(c))), "min {a:?} {b:?} {c:?}");
                assert_eq!(bits(a.max(b).max(c)), bits(a.max(b.max(c))), "max {a:?} {b:?} {c:?}");
            }
        }
    }
}

#[test]
fn clamp_is_max_then_min() {
    let values = interesting();
    for &x in &values {
        for &lo in &values {
            for &hi in &values {
                let expected = match (key(x), key(lo), key(hi)) {
                    (Some(kx), Some(kl), Some(kh)) => {
                        let raised = if kx >= kl { x } else { lo };
                        bits(if kx.max(kl) <= kh { raised } else { hi })
                    }
                    _ => NAN_BITS,
                };
                assert_eq!(bits(x.clamp(lo, hi)), expected, "clamp({x:?}, {lo:?}, {hi:?})");
            }
        }
    }
}

#[test]
fn random_pairs_follow_the_documented_order() {
    let mut rng = Rng(0x00c0_ffee);
    for _ in 0..20_000 {
        let (a, b) = (rng.any(), rng.any());
        assert_eq!(bits(a.min(b)), reference_min(a, b), "min({a:?}, {b:?})");
        assert_eq!(bits(a.max(b)), reference_max(a, b), "max({a:?}, {b:?})");
    }
}

#[test]
fn vectors_select_per_component() {
    let a = Vec3::new(s(1.0), -Scalar::ZERO, Scalar::NAN);
    let b = Vec3::new(s(2.0), Scalar::ZERO, s(3.0));
    assert_eq!(a.min(b).to_array().map(bits), [bits(s(1.0)), NEGATIVE_ZERO, NAN_BITS]);
    assert_eq!(a.max(b).to_array().map(bits), [bits(s(2.0)), POSITIVE_ZERO, NAN_BITS]);
    let clamped = Vec3::splat(s(5.0)).clamp(Vec3::ZERO, Vec3::new(s(1.0), s(9.0), s(5.0)));
    assert_eq!(clamped, Vec3::new(s(1.0), s(5.0), s(5.0)));
}

#[test]
fn comparison_operators_are_ieee() {
    assert!(Scalar::ZERO == -Scalar::ZERO);
    assert!(Scalar::NAN != Scalar::NAN);
    assert!(Scalar::NAN != s(1.0));
    assert_eq!(s(1.0).partial_cmp(&Scalar::NAN), None);
    assert_eq!(Scalar::NAN.partial_cmp(&s(1.0)), None);
    assert!(s(1.0) < s(2.0) && s(2.0) >= s(2.0));
}
