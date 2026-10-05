//! Layout, plain-data traits, canonical bytes, and algebraic sanity of the vector, quaternion,
//! matrix and render-space types. Bit-exact results are pinned by `golden.rs`; the checks
//! here are that the formulas mean what they say.

mod common;

use core::mem::{align_of, size_of};

use bytemuck::{CheckedBitPattern, NoUninit, Pod};
use common::Rng;
use lactose_math::render::RenderPosition;
use lactose_math::{Canonical, Mat3, Mat4, Quat, Scalar, Transform, Vec2, Vec3, Vec4};

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn close(a: Scalar, b: Scalar, tolerance: f64) -> bool {
    (a - b).abs() <= s(tolerance)
}

fn all_close(a: &[Scalar], b: &[Scalar], tolerance: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&a, &b)| close(a, b, tolerance))
}

fn moderate_vec3(rng: &mut Rng) -> Vec3 {
    Vec3::new(rng.with_exponent(-3, 3), rng.with_exponent(-3, 3), rng.with_exponent(-3, 3))
}

fn unit_quat(rng: &mut Rng) -> Quat {
    Quat::new(
        rng.with_exponent(-3, 3),
        rng.with_exponent(-3, 3),
        rng.with_exponent(-3, 3),
        rng.with_exponent(-3, 3),
    )
    .normalize()
}

// Literal constants in `const` context.
const GRAVITY: Scalar = Scalar::from_f64(9.81);
const DOWN: Vec3 = Vec3::new(Scalar::ZERO, Scalar::from_f64(-1.0), Scalar::ZERO);
const HEAVY: bool = GRAVITY.max(Scalar::from_i32(3)).to_i32() == 9;

#[test]
fn constants_are_usable_in_const_context() {
    assert_eq!(GRAVITY.to_f64(), 9.81);
    assert_eq!(DOWN.y, Scalar::NEG_ONE);
    const { assert!(HEAVY) };
    assert_eq!(Scalar::PI.to_f64(), core::f64::consts::PI);
    assert_eq!(Scalar::TAU, Scalar::PI * Scalar::TWO);
}

#[test]
fn layouts_are_padding_free_arrays_of_scalars() {
    fn plain<T: Pod + NoUninit + CheckedBitPattern>(scalars: usize) {
        assert_eq!(size_of::<T>(), scalars * Scalar::BYTES);
        assert_eq!(align_of::<T>(), align_of::<Scalar>());
    }
    plain::<Scalar>(1);
    plain::<Vec2>(2);
    plain::<Vec3>(3);
    plain::<Vec4>(4);
    plain::<Quat>(4);
    plain::<Mat3>(9);
    plain::<Mat4>(16);
    plain::<Transform>(10);

    assert_eq!(size_of::<RenderPosition>(), 24);
    assert_eq!(align_of::<RenderPosition>(), 4);
    let position = RenderPosition { cell: [1, -2, 3], offset: [0.5, 1.5, -2.5] };
    let bytes = bytemuck::bytes_of(&position);
    assert_eq!(bytemuck::pod_read_unaligned::<[i32; 3]>(&bytes[..12]), [1, -2, 3]);
    assert_eq!(bytemuck::pod_read_unaligned::<[f32; 3]>(&bytes[12..]), [0.5, 1.5, -2.5]);
}

#[test]
fn field_order_is_declaration_order() {
    let [a, b, c, d] = [s(1.0), s(2.0), s(3.0), s(4.0)];
    assert_eq!(bytemuck::cast::<Vec3, [Scalar; 3]>(Vec3::new(a, b, c)), [a, b, c]);
    assert_eq!(bytemuck::cast::<Quat, [Scalar; 4]>(Quat::new(a, b, c, d)), [a, b, c, d]);

    let numbered: [Scalar; 16] = core::array::from_fn(|i| Scalar::from_u32(i as u32));
    let mat4 = Mat4::from_array(numbered);
    assert_eq!(bytemuck::cast::<Mat4, [Scalar; 16]>(mat4), numbered);
    assert_eq!(mat4.to_array(), numbered);
    assert_eq!(mat4.y_axis, Vec4::new(s(4.0), s(5.0), s(6.0), s(7.0)));

    let nine: [Scalar; 9] = core::array::from_fn(|i| Scalar::from_u32(i as u32));
    assert_eq!(bytemuck::cast::<Mat3, [Scalar; 9]>(Mat3::from_array(nine)), nine);

    let ten: [Scalar; 10] = core::array::from_fn(|i| Scalar::from_u32(i as u32));
    let transform = Transform::from_array(ten);
    assert_eq!(bytemuck::cast::<Transform, [Scalar; 10]>(transform), ten);
    assert_eq!(transform.rotation, Quat::new(s(3.0), s(4.0), s(5.0), s(6.0)));
    assert_eq!(transform.scale, Vec3::new(s(7.0), s(8.0), s(9.0)));
}

#[test]
fn canonical_bits_collapse_nan_and_keep_signed_zero() {
    let canonical = 0x7ff8_0000_0000_0000_u64;
    for nan in [
        0x7ff8_0000_0000_0000_u64,
        0xfff8_0000_0000_0000,
        0x7ff0_0000_0000_0001,
        0xffff_ffff_ffff_ffff,
        0x7ff8_0000_dead_beef,
    ] {
        // Injected through the float so the payload really is in memory.
        let value = s(f64::from_bits(nan));
        assert!(value.is_nan());
        assert_eq!(value.to_canonical_bits(), canonical);
        assert_eq!(bytemuck::cast::<f64, u64>(value.to_f64()), canonical);
        assert_eq!(bytemuck::cast::<f32, u32>(value.to_f32()), 0x7fc0_0000);
        assert_eq!(value.to_canonical_bytes(), canonical.to_le_bytes());
        assert_eq!(Scalar::from_canonical_bits(nan).to_canonical_bits(), canonical);
        assert_eq!(format!("{value:?} {value}"), "NaN NaN");
    }
    // NaNs made by arithmetic, whose sign differs between x86-64 and aarch64.
    for nan in [Scalar::ZERO / Scalar::ZERO, Scalar::INFINITY - Scalar::INFINITY, s(-1.0).sqrt()] {
        assert_eq!(nan.to_canonical_bits(), canonical);
        assert_eq!((-nan).to_canonical_bits(), canonical);
        assert_eq!(nan.abs().to_canonical_bits(), canonical);
    }

    assert_eq!(Scalar::ZERO.to_canonical_bits(), 0);
    assert_eq!((-Scalar::ZERO).to_canonical_bits(), 1 << 63);
    assert_eq!(Scalar::from_canonical_bits(1 << 63).recip(), Scalar::NEG_INFINITY);
    assert_eq!(s(1.0).to_canonical_bits(), 0x3ff0_0000_0000_0000);
    assert_eq!(s(1.0).to_canonical_bytes(), [0, 0, 0, 0, 0, 0, 0xf0, 0x3f]);
}

#[test]
fn canonical_bytes_round_trip_and_are_scalars_in_order() {
    fn check<T: Canonical + Pod + PartialEq + core::fmt::Debug>(rng: &mut Rng, scalars: usize) {
        for _ in 0..200 {
            let raw: Vec<Scalar> = (0..scalars).map(|_| rng.any()).collect();
            let value: T = bytemuck::pod_read_unaligned(bytemuck::cast_slice(&raw));
            let bytes = value.to_canonical_bytes();

            let expected: Vec<u8> =
                raw.iter().flat_map(|scalar| scalar.to_canonical_bits().to_le_bytes()).collect();
            assert_eq!(bytes.as_ref(), expected);

            // Decoding gives back the canonical value, and encoding is then stable.
            let decoded = T::from_canonical_bytes(bytes);
            assert_eq!(decoded.to_canonical_bytes().as_ref(), expected);
            assert_eq!(value.canonical().to_canonical_bytes().as_ref(), expected);
            // With every NaN canonical, memory and canonical bytes agree on a little-endian
            // target, which is what the bulk-copy save path relies on.
            if cfg!(target_endian = "little") {
                assert_eq!(bytemuck::bytes_of(&value.canonical()), expected);
                assert_eq!(bytemuck::bytes_of(&decoded), expected);
            }
            // The slice route a generic decoder takes.
            let Ok(from_slice) = T::Bytes::try_from(expected.as_slice()) else {
                panic!("the encoding has a fixed length");
            };
            assert_eq!(from_slice.as_ref(), expected);
            assert!(T::Bytes::try_from(&expected[1..]).is_err());
        }
    }
    let mut rng = Rng(0x000c_a001_0ca1);
    check::<Scalar>(&mut rng, 1);
    check::<Vec2>(&mut rng, 2);
    check::<Vec3>(&mut rng, 3);
    check::<Vec4>(&mut rng, 4);
    check::<Quat>(&mut rng, 4);
    check::<Mat3>(&mut rng, 9);
    check::<Mat4>(&mut rng, 16);
    check::<Transform>(&mut rng, 10);
}

#[test]
fn integer_conversions_saturate_and_truncate() {
    assert_eq!(s(2.9).to_i32(), 2);
    assert_eq!(s(-2.9).to_i32(), -2);
    assert_eq!(s(1e30).to_i32(), i32::MAX);
    assert_eq!(s(-1e30).to_i64(), i64::MIN);
    assert_eq!(Scalar::NAN.to_i32(), 0);
    assert_eq!(Scalar::INFINITY.to_i64(), i64::MAX);
    assert_eq!(Scalar::from_i64(-7).to_i64(), -7);
    assert_eq!(Scalar::from_u64(u64::MAX).to_f64(), 18446744073709551616.0);
    assert_eq!(Scalar::from_u32(u32::MAX).to_f64(), 4294967295.0);
    assert_eq!(Scalar::from_f32(0.5).to_f32(), 0.5);
}

#[test]
fn unit_from_bits_covers_the_half_open_unit_interval() {
    assert_eq!(Scalar::unit_from_bits(0), Scalar::ZERO);
    assert_eq!(Scalar::unit_from_bits(0x7ff), Scalar::ZERO);
    assert_eq!(Scalar::unit_from_bits(1 << 63), Scalar::HALF);
    assert_eq!(Scalar::unit_from_bits(1 << 11).to_f64(), 1.0 / 9007199254740992.0);
    let top = Scalar::unit_from_bits(u64::MAX);
    assert!(top < Scalar::ONE);
    assert_eq!(top.to_canonical_bits(), 0x3fef_ffff_ffff_ffff);
    let mut rng = Rng(7);
    for _ in 0..10_000 {
        let unit = Scalar::unit_from_bits(rng.next());
        assert!(unit >= Scalar::ZERO && unit < Scalar::ONE);
    }
}

#[test]
fn scalar_helpers() {
    assert_eq!(s(2.0).powi(10), s(1024.0));
    assert_eq!(s(2.0).powi(-2), s(0.25));
    assert_eq!(s(-3.0).powi(3), s(-27.0));
    assert_eq!(Scalar::NAN.powi(0), Scalar::ONE);
    assert_eq!(s(7.0).powi(i32::MIN), Scalar::ZERO);
    let x = s(1.37);
    assert_eq!(x.powi(5), (x * (x * x) * (x * x)) * Scalar::ONE);

    assert_eq!(s(7.5) % s(2.0), s(1.5));
    assert_eq!(s(-7.5) % s(2.0), s(-1.5));
    assert_eq!(s(-7.5).rem_euclid(s(2.0)), s(0.5));
    assert_eq!(s(-7.5).rem_euclid(s(-2.0)), s(0.5));
    assert!((s(1.0) % Scalar::ZERO).is_nan());

    assert_eq!(s(-2.5).round(), s(-3.0));
    assert_eq!(s(2.5).round(), s(3.0));
    assert_eq!(s(-2.5).floor(), s(-3.0));
    assert_eq!(s(-2.5).ceil(), s(-2.0));
    assert_eq!(s(-2.5).trunc(), s(-2.0));
    assert_eq!(s(-2.5).sign(), Scalar::NEG_ONE);
    assert_eq!((-Scalar::ZERO).sign().to_canonical_bits(), 0);
    assert!(Scalar::NAN.sign().is_nan());

    assert_eq!(s(9.0).sqrt(), s(3.0));
    assert_eq!((-Scalar::ZERO).sqrt().to_canonical_bits(), 1 << 63);
    // Fused: the product is not rounded before the addition.
    let a = s(1.0) + Scalar::EPSILON;
    assert_eq!(a.mul_add(a, s(-1.0)), s(2.0) * Scalar::EPSILON + Scalar::EPSILON * Scalar::EPSILON);
    assert_eq!(a * a + s(-1.0), s(2.0) * Scalar::EPSILON);

    assert_eq!(s(180.0).to_radians(), Scalar::PI);
    assert_eq!(Scalar::PI.to_degrees(), s(180.0));
    assert_eq!(s(2.0).lerp(s(6.0), s(0.25)), s(3.0));
    let (sin, cos) = s(0.7).sin_cos();
    assert_eq!((sin, cos), (s(0.7).sin(), s(0.7).cos()));
}

#[test]
fn vector_basics() {
    let a = Vec3::new(s(1.0), s(2.0), s(3.0));
    let b = Vec3::new(s(-4.0), s(5.0), s(0.5));
    assert_eq!(a.dot(b), s(7.5));
    assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
    assert_eq!(a.cross(b), -b.cross(a));
    assert_eq!(a.cross(b).dot(a), Scalar::ZERO);
    assert_eq!(Vec3::new(s(3.0), s(4.0), s(12.0)).length(), s(13.0));
    assert_eq!(Vec2::new(s(3.0), s(4.0)).length(), s(5.0));
    assert_eq!(Vec2::X.perp_dot(Vec2::Y), Scalar::ONE);
    assert_eq!(a.distance(a), Scalar::ZERO);
    assert_eq!(a * s(2.0), s(2.0) * a);
    assert_eq!(a * b, Vec3::new(s(-4.0), s(10.0), s(1.5)));
    assert_eq!(a.lerp(b, Scalar::ZERO), a);
    assert_eq!(a.truncate().extend(s(3.0)), a);
    assert_eq!(a.extend(s(4.0)).truncate(), a);
    assert_eq!(Vec4::new(s(1.0), s(2.0), s(3.0), s(4.0)).dot(Vec4::ONE), s(10.0));

    assert_eq!(Vec3::new(s(0.0), s(3.0), s(4.0)).normalize(), Vec3::new(s(0.0), s(0.6), s(0.8)));
    assert_eq!(Vec3::ZERO.try_normalize(), None);
    assert_eq!(Vec3::splat(Scalar::INFINITY).try_normalize(), None);
    assert_eq!(Vec3::splat(Scalar::NAN).try_normalize(), None);
    assert!(!Vec3::ZERO.normalize().is_finite());
    assert_eq!(Vec3::new(s(0.0), s(0.0), s(-2.0)).try_normalize(), Some(-Vec3::Z));

    let mut c = a;
    c += b;
    c -= a;
    c *= s(2.0);
    c /= s(2.0);
    assert_eq!(c, b);
}

#[test]
fn quaternion_rotations_are_right_handed() {
    let quarter = Scalar::FRAC_PI_2;
    let tolerance = 1e-15;
    let about_z = Quat::from_rotation_z(quarter);
    assert!(all_close(&(about_z * Vec3::X).to_array(), &Vec3::Y.to_array(), tolerance));
    let about_x = Quat::from_rotation_x(quarter);
    assert!(all_close(&(about_x * Vec3::Y).to_array(), &Vec3::Z.to_array(), tolerance));
    let about_y = Quat::from_rotation_y(quarter);
    assert!(all_close(&(about_y * Vec3::Z).to_array(), &Vec3::X.to_array(), tolerance));

    assert_eq!(Quat::from_axis_angle(Vec3::Z, quarter), about_z);
    assert_eq!(
        Quat::IDENTITY * Vec3::new(s(1.0), s(2.0), s(3.0)),
        Vec3::new(s(1.0), s(2.0), s(3.0))
    );
    // `a * b` applies `b` first.
    let composed = (about_z * about_x) * Vec3::Y;
    assert!(all_close(
        &composed.to_array(),
        &(about_z * (about_x * Vec3::Y)).to_array(),
        tolerance
    ));
}

#[test]
fn quaternion_agrees_with_its_matrix() {
    let mut rng = Rng(0x9a7);
    for _ in 0..500 {
        let (q, p) = (unit_quat(&mut rng), unit_quat(&mut rng));
        let v = moderate_vec3(&mut rng);
        let matrix = Mat3::from_quat(q);
        assert!(all_close(&(q * v).to_array(), &(matrix * v).to_array(), 1e-13));
        assert!(close(matrix.determinant(), Scalar::ONE, 1e-13));
        assert!(all_close(
            &(matrix * Mat3::from_quat(p)).to_array(),
            &Mat3::from_quat(q * p).to_array(),
            1e-13
        ));

        // Back from the matrix: the same rotation, with the canonical sign.
        let back = Quat::from_mat3(matrix);
        assert!(has_canonical_sign(back), "{back:?}");
        assert!(all_close(&back.to_array(), &q.canonical_sign().to_array(), 1e-13));

        assert!(all_close(&(q * q.inverse()).to_array(), &Quat::IDENTITY.to_array(), 1e-14));
        assert!(all_close(&(q.conjugate() * (q * v)).to_array(), &v.to_array(), 1e-13));
        assert!(close(q.length(), Scalar::ONE, 1e-15));
    }
}

/// The documented rule: the first non-zero coefficient in the order w, x, y, z is positive.
fn has_canonical_sign(q: Quat) -> bool {
    [q.w, q.x, q.y, q.z]
        .into_iter()
        .find(|&coefficient| coefficient != Scalar::ZERO)
        .is_none_or(|coefficient| coefficient > Scalar::ZERO)
}

#[test]
fn quaternion_sign_is_canonical() {
    let mut rng = Rng(0x0005_1691);
    for _ in 0..10_000 {
        let q = unit_quat(&mut rng);
        let canonical = q.canonical_sign();
        assert!(has_canonical_sign(canonical), "{canonical:?}");
        assert!(canonical == q || canonical == -q);
        assert_eq!(canonical, (-q).canonical_sign());
        assert_eq!(canonical, canonical.canonical_sign());
        assert!(has_canonical_sign(Quat::from_mat3(Mat3::from_quat(q))));
    }

    // Ties: half turns, where w is zero and a later coefficient decides.
    let (zero, one) = (Scalar::ZERO, Scalar::ONE);
    for (q, expected) in [
        (Quat::new(-one, zero, zero, zero), Quat::new(one, zero, zero, zero)),
        (Quat::new(zero, -one, zero, zero), Quat::new(zero, one, zero, zero)),
        (Quat::new(zero, zero, -one, zero), Quat::new(zero, zero, one, zero)),
        (Quat::new(zero, -one, one, zero), Quat::new(zero, one, -one, zero)),
        (Quat::new(one, -one, -one, zero), Quat::new(one, -one, -one, zero)),
        (Quat::new(one, one, one, -one), Quat::new(-one, -one, -one, one)),
        (Quat::new(-one, zero, zero, -zero), Quat::new(one, zero, zero, zero)),
        (Quat::new(zero, zero, zero, zero), Quat::new(zero, zero, zero, zero)),
    ] {
        assert_eq!(q.canonical_sign(), expected, "{q:?}");
        assert!(has_canonical_sign(q.canonical_sign()));
    }
    // Half-turn matrices, where the conversion's w is zero or rounding noise.
    for axis in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(one, one, zero).normalize(), -Vec3::Z] {
        let half_turn = Mat3::from_quat(Quat::from_axis_angle(axis, Scalar::PI));
        assert!(has_canonical_sign(Quat::from_mat3(half_turn)), "{axis:?}");
    }
    assert_eq!(Quat::from_mat3(Mat3::IDENTITY), Quat::IDENTITY);
    // A NaN in the deciding coefficient leaves the quaternion as it was.
    let nan = Quat::new(-one, zero, zero, Scalar::NAN).canonical_sign();
    assert_eq!(nan.x, -one);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "uniform parent scale")]
fn compose_uniform_rejects_a_non_uniform_parent() {
    let parent = Transform::new(Vec3::ZERO, Quat::IDENTITY, Vec3::new(s(2.0), s(1.0), s(1.0)));
    let _ = parent.compose_uniform(Transform::IDENTITY);
}

#[test]
fn extreme_magnitudes_behave_as_documented() {
    // Squares overflow: the length is infinite and the "normalised" vector is zero.
    let huge = Vec3::splat(s(1e200));
    assert_eq!(huge.normalize(), Vec3::ZERO);
    assert_eq!(huge.try_normalize(), None);
    // Squares underflow: the length is zero.
    let tiny = Vec3::splat(s(1e-200));
    assert!(!tiny.normalize().is_finite());
    assert_eq!(tiny.try_normalize(), None);
    // Invertible, but the determinant leaves the finite range.
    assert_eq!(Mat3::from_diagonal(Vec3::splat(s(1e-120))).inverse(), None);
    assert_eq!(Mat3::from_diagonal(Vec3::splat(s(1e120))).inverse(), None);
    assert_eq!((Mat4::IDENTITY * s(1e-90)).inverse(), None);
    // The reciprocal of an overflowed positive power.
    assert_eq!(s(2.0).powi(-1074), Scalar::ZERO);
    assert_eq!(s(2.0).powf(s(-1074.0)).to_canonical_bits(), 1);
}

#[test]
fn quaternion_interpolation() {
    let mut rng = Rng(0x51e9);
    for _ in 0..500 {
        let (a, b) = (unit_quat(&mut rng), unit_quat(&mut rng));
        let shorter = if a.dot(b) < Scalar::ZERO { -b } else { b };
        for interpolate in [Quat::slerp, Quat::nlerp] {
            assert!(all_close(&interpolate(a, b, Scalar::ZERO).to_array(), &a.to_array(), 1e-14));
            assert!(all_close(
                &interpolate(a, b, Scalar::ONE).to_array(),
                &shorter.to_array(),
                1e-14
            ));
            assert!(close(interpolate(a, b, s(0.3)).length(), Scalar::ONE, 1e-14));
        }
        // Constant angular speed: halfway is equally far from both ends.
        let half = a.slerp(b, Scalar::HALF);
        assert!(close(half.dot(a), half.dot(shorter), 1e-14));
    }
    // Nearly parallel inputs take the linear branch and stay finite.
    let a = Quat::IDENTITY;
    let b = Quat::from_rotation_y(s(1e-9));
    assert!(a.slerp(b, s(0.5)).is_finite());
    assert!(all_close(&a.slerp(a, s(0.5)).to_array(), &a.to_array(), 0.0));
}

#[test]
fn matrix_products_and_inverses() {
    let mut rng = Rng(0x3a7);
    for _ in 0..500 {
        let m3 = Mat3::from_cols(
            moderate_vec3(&mut rng),
            moderate_vec3(&mut rng),
            moderate_vec3(&mut rng),
        );
        let v = moderate_vec3(&mut rng);
        assert_eq!(m3 * Mat3::IDENTITY, m3);
        assert_eq!(Mat3::IDENTITY * m3, m3);
        assert_eq!(m3.transpose().transpose(), m3);
        assert_eq!((m3 * Mat3::from_diagonal(Vec3::splat(s(2.0)))) * v, m3 * (v * s(2.0)));
        assert_eq!(m3 + m3, m3 * s(2.0));
        assert_eq!(m3 - m3, Mat3::ZERO);
        if m3.determinant().abs() > s(1e-3) {
            let inverse = m3.inverse().expect("determinant is not zero");
            assert!(all_close(&(m3 * inverse).to_array(), &Mat3::IDENTITY.to_array(), 1e-9));
            assert!(close(m3.transpose().determinant(), m3.determinant(), 1e-9));
        }

        let numbers: [Scalar; 16] = core::array::from_fn(|_| rng.with_exponent(-3, 3));
        let m4 = Mat4::from_array(numbers);
        assert_eq!(m4 * Mat4::IDENTITY, m4);
        assert_eq!(Mat4::IDENTITY * m4, m4);
        assert_eq!(m4.transpose().transpose(), m4);
        assert_eq!(m4 * s(1.0), m4);
        if m4.determinant().abs() > s(1e-3) {
            let inverse = m4.inverse().expect("determinant is not zero");
            assert!(all_close(&(m4 * inverse).to_array(), &Mat4::IDENTITY.to_array(), 1e-7));
            assert!(all_close(&(inverse * m4).to_array(), &Mat4::IDENTITY.to_array(), 1e-7));
            assert!(close(inverse.determinant() * m4.determinant(), Scalar::ONE, 1e-7));
        }
    }
    assert_eq!(Mat3::ZERO.inverse(), None);
    assert_eq!(Mat4::ZERO.inverse(), None);
    assert_eq!(Mat3::from_diagonal(Vec3::splat(Scalar::INFINITY)).inverse(), None);
    assert_eq!(Mat4::IDENTITY.inverse(), Some(Mat4::IDENTITY));
    assert_eq!(Mat3::IDENTITY.inverse(), Some(Mat3::IDENTITY));
    assert_eq!(Mat4::IDENTITY.determinant(), Scalar::ONE);
    // A matrix with a known determinant: the block diagonal of [[1,2],[3,4]] and [[5,6],[7,8]].
    let block = Mat4::from_array(
        [1.0, 3.0, 0.0, 0.0, 2.0, 4.0, 0.0, 0.0, 0.0, 0.0, 5.0, 7.0, 0.0, 0.0, 6.0, 8.0].map(s),
    );
    assert_eq!(block.determinant(), s(4.0));
}

#[test]
fn transform_matches_its_matrix() {
    let mut rng = Rng(0x7f);
    for _ in 0..500 {
        let transform =
            Transform::new(moderate_vec3(&mut rng), unit_quat(&mut rng), moderate_vec3(&mut rng));
        let point = moderate_vec3(&mut rng);
        let matrix = transform.to_mat4();
        assert!(all_close(
            &transform.transform_point(point).to_array(),
            &matrix.transform_point3(point).to_array(),
            1e-12
        ));
        assert!(all_close(
            &transform.transform_vector(point).to_array(),
            &matrix.transform_vector3(point).to_array(),
            1e-12
        ));
        assert!(all_close(
            &(matrix * point.extend(Scalar::ONE)).to_array(),
            &matrix.transform_point3(point).extend(Scalar::ONE).to_array(),
            0.0
        ));

        // Composition under a uniform parent scale is the matrix product.
        let parent =
            Transform::new(moderate_vec3(&mut rng), unit_quat(&mut rng), Vec3::splat(s(1.5)));
        let composed = parent.compose_uniform(transform);
        assert!(all_close(
            &composed.to_mat4().to_array(),
            &(parent.to_mat4() * matrix).to_array(),
            1e-11
        ));
        assert!(all_close(
            &composed.transform_point(point).to_array(),
            &parent.transform_point(transform.transform_point(point)).to_array(),
            1e-11
        ));
    }
    let here = Vec3::new(s(1.0), s(2.0), s(3.0));
    assert_eq!(Transform::at(here).transform_point(Vec3::ZERO), here);
    assert_eq!(Transform::IDENTITY.compose_uniform(Transform::at(here)), Transform::at(here));
    assert_eq!(Transform::IDENTITY.to_mat4(), Mat4::IDENTITY);
}

#[test]
fn render_position_splits_into_cell_and_offset() {
    let size = f64::from(RenderPosition::CELL_SIZE);
    let position = RenderPosition::from_sim(Vec3::new(s(0.0), s(100.0), s(-100.0)));
    assert_eq!(position.cell, [0, 2, -2]);
    assert_eq!(position.offset, [0.0, -28.0, 28.0]);
    // A cell's edge belongs to the cell above it.
    assert_eq!(RenderPosition::from_sim(Vec3::splat(s(32.0))).cell, [1; 3]);
    assert_eq!(RenderPosition::from_sim(Vec3::splat(s(32.0))).offset, [-32.0; 3]);
    assert_eq!(RenderPosition::from_sim(Vec3::splat(s(-32.0))).cell, [0; 3]);

    let mut rng = Rng(0x12e9de12);
    for _ in 0..5_000 {
        let sim = Vec3::new(
            rng.with_exponent(-20, 30),
            rng.with_exponent(-20, 30),
            rng.with_exponent(-20, 30),
        );
        let position = RenderPosition::from_sim(sim);
        for axis in 0..3 {
            let offset = f64::from(position.offset[axis]);
            assert!((-size / 2.0..=size / 2.0).contains(&offset));
            let rebuilt = f64::from(position.cell[axis]) * size + offset;
            // The only loss is narrowing the offset to 32 bits.
            assert!((rebuilt - sim.to_array()[axis].to_f64()).abs() <= 2e-6);
        }
        // Relative to its own cell a position is its offset; one cell away it is a cell further.
        assert_eq!(position.relative_to(position.cell), position.offset);
        let shifted =
            position.relative_to([position.cell[0] - 1, position.cell[1], position.cell[2] + 1]);
        assert_eq!(shifted[0], position.offset[0] + RenderPosition::CELL_SIZE);
        assert_eq!(shifted[2], position.offset[2] - RenderPosition::CELL_SIZE);
    }

    // Beyond the grid the cell saturates rather than wrapping.
    assert_eq!(RenderPosition::from_sim(Vec3::splat(s(1e300))).cell, [i32::MAX; 3]);
    assert_eq!(RenderPosition::from_sim(Vec3::splat(s(-1e300))).cell, [i32::MIN; 3]);
    let nan = RenderPosition::from_sim(Vec3::splat(Scalar::NAN));
    assert_eq!(nan.cell, [0; 3]);
    assert!(nan.offset.iter().all(|offset| offset.is_nan()));
}
