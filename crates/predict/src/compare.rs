//! Whether a predicted component agrees with the server's (`README.md`, "Reconciliation"):
//! the one function that decides it, so that a later decision on what prediction compares
//! (a tolerance for dynamic bodies, say) changes this module and nothing else.

use lactose_math::Scalar;
use lactose_physics_api::Teleported;
use lactose_reflect::{Field, FieldInfo, FieldKind, Reflect, TypeInfo};

/// How one component of a fact compares with the record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Comparison {
    /// Equal over every field with an ID.
    Agrees,
    /// Present, and `fields` of its fields with an ID differ.
    Differs { fields: u32 },
    /// The record lacks the component.
    Missing,
    /// Neither compared nor written by a correction: [`Teleported`]. A corrected `Transform`
    /// reaches the backend as a placement, which the write-back reports as a teleport on the
    /// client while the server's flag is unchanged, so comparing it would correct every
    /// correction again one frame later.
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
/// negative zero is its own. Full precision, no tolerance.
pub(crate) fn compare(info: &TypeInfo, predicted: Option<&[u8]>, server: &[u8]) -> Comparison {
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
    if fields == 0 { Comparison::Agrees } else { Comparison::Differs { fields } }
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
    use lactose_math::{Scalar, Vec3};
    use lactose_physics_api::Teleported;
    use lactose_reflect::{Padding, Reflect, TypeInfo, bytemuck, component};

    use super::{Comparison, compare};

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
        compare(info(), Some(&bytes(&golden())), &bytes(&server))
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
        assert_eq!(compare(info(), Some(&canonical), &server), Comparison::Agrees);
        let server = patched(&nan, 1, Scalar::BYTES, &other);
        assert_eq!(compare(info(), Some(&canonical), &server), Comparison::Agrees);
        let server = patched(&nan, 3, Scalar::BYTES, &other);
        assert_eq!(compare(info(), Some(&canonical), &server), Comparison::Agrees);
        // A NaN is not a number it could be confused with.
        assert_eq!(
            compare(info(), Some(&bytes(&golden())), &server),
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
            compare(info(), Some(&bytes(&zero)), &bytes(&negative)),
            Comparison::Differs { fields: 1 }
        );
    }

    #[test]
    fn padding_is_not_compared() {
        let value = golden();
        let padding = info().layout().fields().iter().find(|field| field.is_padding()).unwrap();
        let mut server = bytes(&value);
        server[padding.offset()] = 0xff;
        assert_eq!(compare(info(), Some(&bytes(&value)), &server), Comparison::Agrees);
    }

    #[test]
    fn a_missing_record_and_a_wrong_length() {
        let value = bytes(&golden());
        assert_eq!(compare(info(), None, &value), Comparison::Missing);
        assert_eq!(compare(info(), Some(&value), &value[1..]), Comparison::Malformed);
        assert_eq!(compare(info(), None, &[]), Comparison::Malformed);
    }

    #[test]
    fn teleported_is_never_compared() {
        let predicted = bytemuck::bytes_of(&Teleported(Tick(16))).to_vec();
        let server = bytemuck::bytes_of(&Teleported(Tick(2))).to_vec();
        assert_eq!(
            compare(Teleported::SCHEMA.info(), Some(&predicted), &server),
            Comparison::Ignored
        );
        assert_eq!(compare(Teleported::SCHEMA.info(), None, &server), Comparison::Ignored);
    }
}
