//! Whether a predicted component agrees with the server's (`README.md`, "Reconciliation"):
//! the one function that decides it, the tolerance for a dynamic body's solver-written rows
//! included (D124), and the tolerance itself.

use core::cmp::Ordering;

use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Teleported, Transform, Velocity};
use lactose_reflect::{Field, FieldInfo, FieldKind, Reflect, TypeInfo, decode};

use crate::reconcile::Absorbed;

/// How far a dynamic body's solver-written [`Transform`] and [`Velocity`] may differ from
/// the server's, per scalar component, and still agree (D124): the physics solver holds
/// state the rows do not carry, warm starts, contact manifolds and sleep, which the
/// client's solver reaches by another path after a correction, a claim or a rebuild. Every
/// other field, and these of a kinematic or fixed body, agrees by canonical bytes only.
///
/// A predictor starts at [`DEFAULT`](Self::DEFAULT); `app` replaces it between runs with
/// [`Predictor::set_tolerance`](crate::Predictor::set_tolerance). A bound that is negative
/// or NaN absorbs nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PredictionTolerance {
    /// Per component of `Transform::translation`, in world units (metres).
    pub translation: Scalar,
    /// Per component of the unit quaternion `Transform::rotation`.
    pub rotation: Scalar,
    /// Per component of `Velocity::linear`, in units per second.
    pub linear: Scalar,
    /// Per component of `Velocity::angular`, in radians per second.
    pub angular: Scalar,
}

impl PredictionTolerance {
    /// 2e-3 m, 2e-3 per quaternion component, 2e-2 m/s and 2e-2 rad/s: under a pixel at
    /// five metres on a 1440p display with a 60 degree field of view, set under the 64-bit
    /// scalar against the residuals D124 measured. Milestone 1 re-measures them with the
    /// scalar decision and the spike re-sets them from `PredictionStats::absorbed`.
    pub const DEFAULT: Self = Self {
        translation: Scalar::from_f64(2e-3),
        rotation: Scalar::from_f64(2e-3),
        linear: Scalar::from_f64(2e-2),
        angular: Scalar::from_f64(2e-2),
    };
}

/// How one component of a fact compares with the record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Comparison {
    /// Equal over every field with an ID.
    Agrees,
    /// Present, and `fields` of its fields with an ID differ.
    Differs { fields: u32 },
    /// Present, its bytes differ, and every scalar leaf is within the tolerance: a dynamic
    /// body's `Transform` or `Velocity`. Agrees for the comparison; a correction made for
    /// another fact writes it all the same, which is what converges (D124).
    Absorbed(Absorbed),
    /// The record lacks the component.
    Missing,
    /// Neither compared nor written by a correction: [`Teleported`], the client's own
    /// no-interpolate flag. A corrected `Transform` reaches the backend as a placement,
    /// which the write-back reports as a teleport on the client while the server's flag is
    /// unchanged, so comparing it would correct every correction again one frame later. The
    /// predictor never records it, so it never reaches here from a fact.
    Ignored,
    /// The server's bytes are not one value of the component: a defect of the sender.
    Malformed,
}

/// Compares `server`, the bytes of one value of the component `info` describes, with
/// `predicted`, the record's bytes of it or `None` if the record lacks it.
///
/// Per field, over every field with an ID, by canonical bytes: an integer, `bool`, enum,
/// identity newtype, entity reference or asset reference by its memory bytes (storage holds
/// origin keys on both machines), and a leaf tagged as `lactose_math::Scalar`, at any depth
/// of a nested struct or array, by its canonical bits, so every NaN is one value and
/// negative zero is its own. Full precision, with one exception: `tolerance` is `Some` when
/// the caller found the entity's body dynamic (`README.md`, "Reconciliation"), and then a
/// `Transform` or `Velocity` whose bytes differ is [`Comparison::Absorbed`] when every
/// scalar leaf is within it.
pub(crate) fn compare(
    info: &TypeInfo,
    predicted: Option<&[u8]>,
    server: &[u8],
    tolerance: Option<&PredictionTolerance>,
) -> Comparison {
    if info.key() == Teleported::KEY {
        return Comparison::Ignored;
    }
    let size = info.layout().size();
    if server.len() != size {
        return Comparison::Malformed;
    }
    let Some(predicted) = predicted.filter(|bytes| bytes.len() == size) else {
        return Comparison::Missing;
    };
    let mut fields = 0;
    for field in info.layout().fields() {
        if !field.is_padding() && !field_equal(field, predicted, server) {
            fields += 1;
        }
    }
    if fields == 0 {
        return Comparison::Agrees;
    }
    match tolerance.and_then(|tolerance| absorb(info, predicted, server, tolerance)) {
        Some(absorbed) => Comparison::Absorbed(absorbed),
        None => Comparison::Differs { fields },
    }
}

/// What the tolerance absorbs of a `Transform` or `Velocity` whose bytes differ, or `None`
/// if a leaf is beyond it or the component is neither.
fn absorb(
    info: &TypeInfo,
    predicted: &[u8],
    server: &[u8],
    tolerance: &PredictionTolerance,
) -> Option<Absorbed> {
    let mut absorbed = Absorbed::default();
    if info.key() == Transform::KEY {
        let (a, b) = (decode::<Transform>(predicted).ok()?, decode::<Transform>(server).ok()?);
        let largest = &mut absorbed.translation;
        let field =
            leaves(&vec3(a.translation), &vec3(b.translation), tolerance.translation, largest)?;
        let largest = &mut absorbed.rotation;
        let other = leaves(&quat(a.rotation), &quat(b.rotation), tolerance.rotation, largest)?;
        absorbed.fields = u64::from(field) + u64::from(other);
    } else if info.key() == Velocity::KEY {
        let (a, b) = (decode::<Velocity>(predicted).ok()?, decode::<Velocity>(server).ok()?);
        let largest = &mut absorbed.linear;
        let field = leaves(&vec3(a.linear), &vec3(b.linear), tolerance.linear, largest)?;
        let largest = &mut absorbed.angular;
        let other = leaves(&vec3(a.angular), &vec3(b.angular), tolerance.angular, largest)?;
        absorbed.fields = u64::from(field) + u64::from(other);
    } else {
        return None;
    }
    Some(absorbed)
}

/// Whether one field's leaves differ, each within `bound`, keeping the largest difference
/// in `largest`; `None` if one is beyond it. A leaf whose canonical bits are equal agrees,
/// so a NaN agrees with a NaN and with nothing else: its difference is NaN, never within.
fn leaves(
    predicted: &[Scalar],
    server: &[Scalar],
    bound: Scalar,
    largest: &mut u64,
) -> Option<bool> {
    let mut differs = false;
    for (&a, &b) in predicted.iter().zip(server) {
        if a.to_canonical_bits() == b.to_canonical_bits() {
            continue;
        }
        let difference = (a - b).abs();
        if !matches!(difference.partial_cmp(&bound), Some(Ordering::Less | Ordering::Equal)) {
            return None;
        }
        differs = true;
        // A difference is never negative, so its bits order as its value does.
        #[allow(
            clippy::useless_conversion,
            reason = "`ScalarBits` is `u32` under the 32-bit scalar"
        )]
        let bits = u64::from(difference.to_canonical_bits());
        *largest = (*largest).max(bits);
    }
    Some(differs)
}

const fn vec3(v: Vec3) -> [Scalar; 3] {
    [v.x, v.y, v.z]
}

const fn quat(q: Quat) -> [Scalar; 4] {
    [q.x, q.y, q.z, q.w]
}

/// Whether `field` of the two values `a` and `b` of its parent agrees.
fn field_equal(field: &FieldInfo, a: &[u8], b: &[u8]) -> bool {
    let range = field.offset()..field.offset().saturating_add(field.size());
    match (a.get(range.clone()), b.get(range)) {
        (Some(a), Some(b)) => kind_equal(&field.kind(), field.type_name(), a, b),
        _ => false,
    }
}

fn kind_equal(kind: &FieldKind, tag: &str, a: &[u8], b: &[u8]) -> bool {
    match kind {
        FieldKind::Padding => true,
        FieldKind::Scalar { .. } if tag == <Scalar as Field>::NAME => {
            canonical_bits(a).is_some_and(|bits| Some(bits) == canonical_bits(b))
        }
        FieldKind::Struct(info) => {
            info.fields().iter().all(|field| field.is_padding() || field_equal(field, a, b))
        }
        FieldKind::Array { element, element_name, len } => match a.len().checked_div(*len) {
            Some(width) if width > 0 && a.len() == b.len() => a
                .chunks_exact(width)
                .zip(b.chunks_exact(width))
                .all(|(a, b)| kind_equal(element, element_name, a, b)),
            _ => a == b,
        },
        _ => a == b,
    }
}

/// The canonical bits of an IEEE binary32 or binary64 value from its memory bytes: the
/// bits themselves, with every NaN folded to one pattern. Integer work only.
fn canonical_bits(bytes: &[u8]) -> Option<u64> {
    const NAN: u64 = u64::MAX;
    match *bytes {
        [a, b, c, d] => {
            let bits = u32::from_ne_bytes([a, b, c, d]);
            let nan = bits & 0x7f80_0000 == 0x7f80_0000 && bits & 0x007f_ffff != 0;
            Some(if nan { NAN } else { u64::from(bits) })
        }
        [a, b, c, d, e, f, g, h] => {
            let bits = u64::from_ne_bytes([a, b, c, d, e, f, g, h]);
            let exponent = 0x7ff0_0000_0000_0000;
            let nan = bits & exponent == exponent && bits & 0x000f_ffff_ffff_ffff != 0;
            Some(if nan { NAN } else { bits })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use lactose_core::{FieldId, PlainArray, Tick};
    use lactose_math::{Quat, Scalar, Vec3};
    use lactose_physics_api::{Teleported, Transform, Velocity};
    use lactose_reflect::{Padding, Reflect, TypeInfo, bytemuck, component};

    use super::{Comparison, PredictionTolerance, compare};
    use crate::reconcile::Absorbed;

    /// One field of every kind a comparison distinguishes: a nested struct of scalars, a
    /// scalar, an array of scalars, an integer, a `bool`, and padding.
    #[component(key = "compare::golden", persistent, replicated)]
    #[derive(Debug)]
    struct Golden {
        #[field(id = 1)]
        position: Vec3,
        #[field(id = 2)]
        speed: Scalar,
        #[field(id = 3)]
        path: PlainArray<Scalar, 2>,
        #[field(id = 4)]
        hits: u32,
        #[field(id = 5)]
        alive: bool,
        #[field(padding)]
        _pad: Padding<3>,
    }

    fn golden() -> Golden {
        Golden {
            position: Vec3::new(Scalar::ONE, Scalar::TWO, Scalar::HALF),
            speed: Scalar::from_i32(3),
            path: PlainArray([Scalar::ZERO, Scalar::NEG_ONE]),
            hits: 7,
            alive: true,
            _pad: Padding::default(),
        }
    }

    fn info() -> &'static TypeInfo {
        Golden::SCHEMA.info()
    }

    fn bytes(value: &Golden) -> Vec<u8> {
        bytemuck::bytes_of(value).to_vec()
    }

    fn compare_with(change: impl FnOnce(&mut Golden)) -> Comparison {
        let mut server = golden();
        change(&mut server);
        compare(info(), Some(&bytes(&golden())), &bytes(&server), None)
    }

    /// A NaN whose payload is not the canonical one, as the memory bytes of a `Scalar`.
    fn other_nan() -> Vec<u8> {
        match Scalar::BYTES {
            4 => 0x7fc0_0001_u32.to_ne_bytes().to_vec(),
            _ => 0x7ff8_0000_0000_0001_u64.to_ne_bytes().to_vec(),
        }
    }

    /// `value`'s bytes with the field `id` at its offset plus `within` overwritten.
    fn patched(value: &Golden, id: u16, within: usize, with: &[u8]) -> Vec<u8> {
        let field =
            info().layout().fields().iter().find(|field| field.id() == Some(FieldId(id))).unwrap();
        let mut bytes = bytes(value);
        let at = field.offset() + within;
        bytes[at..at + with.len()].copy_from_slice(with);
        bytes
    }

    #[test]
    fn equal_bytes_agree() {
        assert_eq!(compare_with(|_| {}), Comparison::Agrees);
    }

    #[test]
    fn each_field_kind_differing_is_one_field() {
        assert_eq!(compare_with(|g| g.hits = 8), Comparison::Differs { fields: 1 });
        assert_eq!(compare_with(|g| g.alive = false), Comparison::Differs { fields: 1 });
        assert_eq!(compare_with(|g| g.speed = Scalar::ONE), Comparison::Differs { fields: 1 });
        // Inside a nested struct and inside an array: the top-level field counts once.
        assert_eq!(compare_with(|g| g.position.y = Scalar::ONE), Comparison::Differs { fields: 1 });
        let both = |g: &mut Golden| {
            g.position.x = Scalar::ZERO;
            g.position.z = Scalar::ZERO;
        };
        assert_eq!(compare_with(both), Comparison::Differs { fields: 1 });
        assert_eq!(compare_with(|g| g.path.0[1] = Scalar::ONE), Comparison::Differs { fields: 1 });
        let two = |g: &mut Golden| {
            g.hits = 0;
            g.path.0[0] = Scalar::ONE;
        };
        assert_eq!(compare_with(two), Comparison::Differs { fields: 2 });
    }

    #[test]
    fn a_scalar_leafs_nan_with_another_payload_agrees() {
        let mut nan = golden();
        nan.speed = Scalar::NAN;
        nan.position.y = Scalar::NAN;
        nan.path.0[1] = Scalar::NAN;
        let canonical = bytes(&nan);
        let other = other_nan();
        let server = patched(&nan, 2, 0, &other);
        assert_ne!(server, canonical);
        assert_eq!(compare(info(), Some(&canonical), &server, None), Comparison::Agrees);
        let server = patched(&nan, 1, Scalar::BYTES, &other);
        assert_eq!(compare(info(), Some(&canonical), &server, None), Comparison::Agrees);
        let server = patched(&nan, 3, Scalar::BYTES, &other);
        assert_eq!(compare(info(), Some(&canonical), &server, None), Comparison::Agrees);
        // A NaN is not a number it could be confused with.
        assert_eq!(
            compare(info(), Some(&bytes(&golden())), &server, None),
            Comparison::Differs { fields: 3 }
        );
    }

    #[test]
    fn negative_zero_is_its_own_value() {
        let mut zero = golden();
        zero.speed = Scalar::ZERO;
        let mut negative = golden();
        negative.speed = Scalar::from_f64(-0.0);
        assert_eq!(
            compare(info(), Some(&bytes(&zero)), &bytes(&negative), None),
            Comparison::Differs { fields: 1 }
        );
    }

    #[test]
    fn padding_is_not_compared() {
        let value = golden();
        let padding = info().layout().fields().iter().find(|field| field.is_padding()).unwrap();
        let mut server = bytes(&value);
        server[padding.offset()] = 0xff;
        assert_eq!(compare(info(), Some(&bytes(&value)), &server, None), Comparison::Agrees);
    }

    #[test]
    fn a_missing_record_and_a_wrong_length() {
        let value = bytes(&golden());
        assert_eq!(compare(info(), None, &value, None), Comparison::Missing);
        assert_eq!(compare(info(), Some(&value), &value[1..], None), Comparison::Malformed);
        assert_eq!(compare(info(), None, &[], None), Comparison::Malformed);
    }

    #[test]
    fn teleported_is_never_compared() {
        let predicted = bytemuck::bytes_of(&Teleported(Tick(16))).to_vec();
        let server = bytemuck::bytes_of(&Teleported(Tick(2))).to_vec();
        assert_eq!(
            compare(Teleported::SCHEMA.info(), Some(&predicted), &server, None),
            Comparison::Ignored
        );
        assert_eq!(compare(Teleported::SCHEMA.info(), None, &server, None), Comparison::Ignored);
    }

    // ---- The tolerance (D124) ----

    const TOLERANCE: PredictionTolerance = PredictionTolerance::DEFAULT;

    fn f(value: f64) -> Scalar {
        Scalar::from_f64(value)
    }

    fn body() -> Transform {
        Transform::new(Vec3::new(f(1.0), f(2.0), f(3.0)), Quat::IDENTITY)
    }

    /// The comparison of a `Transform` with `body()`, the predicted one, under `tolerance`.
    fn transform(server: Transform, tolerance: Option<&PredictionTolerance>) -> Comparison {
        let info = Transform::SCHEMA.info();
        compare(info, Some(bytemuck::bytes_of(&body())), bytemuck::bytes_of(&server), tolerance)
    }

    fn canonical(value: Scalar) -> u64 {
        #[allow(
            clippy::useless_conversion,
            reason = "`ScalarBits` is `u32` under the 32-bit scalar"
        )]
        u64::from(value.to_canonical_bits())
    }

    fn bits(value: f64) -> u64 {
        canonical(f(value))
    }

    #[test]
    fn a_dynamic_bodys_transform_within_the_tolerance_is_absorbed_with_its_largest_difference() {
        let mut server = body();
        server.translation.x = f(1.0) + f(1e-3);
        server.translation.z = f(3.0) - f(2e-3);
        server.rotation.w = f(1.0) - f(5e-4);
        let difference = |a: Scalar, b: Scalar| canonical((a - b).abs());
        let expected = Absorbed {
            fields: 2,
            translation: difference(f(3.0), server.translation.z),
            rotation: difference(f(1.0), server.rotation.w),
            ..Absorbed::default()
        };
        assert!(expected.translation > expected.rotation);
        assert_eq!(transform(server, Some(&TOLERANCE)), Comparison::Absorbed(expected));
        // Equal bytes are not absorbed: they agree.
        assert_eq!(transform(body(), Some(&TOLERANCE)), Comparison::Agrees);
    }

    #[test]
    fn one_leaf_beyond_the_tolerance_is_a_difference_of_every_differing_field() {
        let mut server = body();
        server.translation.x = f(1.0) + f(1e-3);
        server.rotation.y = f(3e-3);
        assert_eq!(transform(server, Some(&TOLERANCE)), Comparison::Differs { fields: 2 });
        let mut server = body();
        server.translation.y = f(2.0) + f(2.5e-3);
        assert_eq!(transform(server, Some(&TOLERANCE)), Comparison::Differs { fields: 1 });
    }

    #[test]
    fn without_the_tolerance_a_transform_agrees_by_bytes_only() {
        // What the caller passes for a kinematic or fixed body.
        let mut server = body();
        server.translation.x = f(1.0) + f(1e-9);
        assert_eq!(transform(server, None), Comparison::Differs { fields: 1 });
    }

    #[test]
    fn a_velocity_within_the_tolerance_is_absorbed_per_kind() {
        let info = Velocity::SCHEMA.info();
        let predicted = Velocity { linear: Vec3::new(f(1.0), f(0.0), f(0.0)), angular: Vec3::ZERO };
        let mut server = predicted;
        server.linear.x = f(1.0) + f(1e-2);
        server.angular.y = f(-2e-2);
        let compared = compare(
            info,
            Some(bytemuck::bytes_of(&predicted)),
            bytemuck::bytes_of(&server),
            Some(&TOLERANCE),
        );
        let Comparison::Absorbed(absorbed) = compared else { panic!("{compared:?}") };
        assert_eq!(absorbed.fields, 2);
        assert_eq!(absorbed.angular, bits(2e-2));
        assert!(absorbed.linear > 0 && absorbed.linear < bits(2e-2));
        assert_eq!((absorbed.translation, absorbed.rotation), (0, 0));
        // At the bound is within; just past it is not.
        server.angular.y = f(-2e-2) - f(1e-9);
        let compared = compare(
            info,
            Some(bytemuck::bytes_of(&predicted)),
            bytemuck::bytes_of(&server),
            Some(&TOLERANCE),
        );
        assert_eq!(compared, Comparison::Differs { fields: 2 });
    }

    #[test]
    fn a_nan_leaf_agrees_with_an_equal_nan_only() {
        let mut nan = body();
        nan.translation.x = Scalar::NAN;
        // Another payload is the same NaN: equal bytes after canonicalisation.
        let mut server = bytemuck::bytes_of(&nan).to_vec();
        let other = other_nan();
        server[..other.len()].copy_from_slice(&other);
        let info = Transform::SCHEMA.info();
        let compared = compare(info, Some(bytemuck::bytes_of(&nan)), &server, Some(&TOLERANCE));
        assert_eq!(compared, Comparison::Agrees);
        // A NaN against a number, either way, is beyond any tolerance.
        assert_eq!(transform(nan, Some(&TOLERANCE)), Comparison::Differs { fields: 1 });
        let compared = compare(
            info,
            Some(bytemuck::bytes_of(&nan)),
            bytemuck::bytes_of(&body()),
            Some(&TOLERANCE),
        );
        assert_eq!(compared, Comparison::Differs { fields: 1 });
        let mut infinite = body();
        infinite.translation.x = Scalar::INFINITY;
        assert_eq!(transform(infinite, Some(&TOLERANCE)), Comparison::Differs { fields: 1 });
    }

    #[test]
    fn negative_zero_within_the_tolerance_is_absorbed_with_no_difference() {
        let mut zero = body();
        zero.rotation.x = f(-0.0);
        let compared = transform(zero, Some(&TOLERANCE));
        assert_eq!(compared, Comparison::Absorbed(Absorbed { fields: 1, ..Absorbed::default() }));
    }

    #[test]
    fn the_tolerance_is_for_transform_and_velocity_alone_and_a_nan_bound_absorbs_nothing() {
        let compared = compare(
            info(),
            Some(&bytes(&golden())),
            &patched(&golden(), 2, 0, bytemuck::bytes_of(&(f(3.0) + f(1e-9)))),
            Some(&TOLERANCE),
        );
        assert_eq!(compared, Comparison::Differs { fields: 1 });
        let mut server = body();
        server.translation.x = f(1.0) + f(1e-9);
        let nan = PredictionTolerance { translation: Scalar::NAN, ..TOLERANCE };
        assert_eq!(transform(server, Some(&nan)), Comparison::Differs { fields: 1 });
        let negative = PredictionTolerance { translation: f(-1.0), ..TOLERANCE };
        assert_eq!(transform(server, Some(&negative)), Comparison::Differs { fields: 1 });
    }
}
