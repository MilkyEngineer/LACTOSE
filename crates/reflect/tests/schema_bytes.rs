//! `TypeInfo::write_schema` is the byte encoding its documentation states: a golden
//! encoding for a small type, byte for byte, and the extras an enum kind appends.

#![forbid(unsafe_code)]

use lactose_reflect::{Field, Padding, Reflect, component, value};

/// Two fields and padding; transient and replicated.
#[component(key = "test::golden", transient, replicated)]
struct Golden {
    #[field(id = 1)]
    count: u32,
    #[field(id = 2)]
    on: bool,
    #[field(padding)]
    _pad: Padding<3>,
}

/// A fieldless enum with a gap in its values.
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq, Debug)]
enum Two {
    /// Zero.
    A = 0,
    /// Three.
    B = 3,
}

/// One enum field; derived and untracked.
#[component(key = "test::golden_enum", derived)]
struct GoldenEnum {
    #[field(id = 7)]
    two: Two,
}

#[test]
fn write_schema_is_the_documented_byte_encoding() {
    let mut bytes = Vec::new();
    Golden::SCHEMA.info().write_schema(&mut bytes);
    let mut expected = Golden::KEY.0.to_le_bytes().to_vec();
    expected.extend_from_slice(&[
        1, 0, 0, 0, // version 1
        0, // a component
        1, // transient
        1, // replicated, not extracted
        8, 0, 0, 0, // size
        4, 0, 0, 0, // alignment
        2, 0, 0, 0, // two fields: the padding is not one
        1, 0, 0, 0, 0, 0, 4, 0, 0, 0, // id 1 at offset 0, four bytes
        3, 0, 0, 0, b'u', b'3', b'2', // its tag
        1,    // unsigned
        2, 0, 4, 0, 0, 0, 1, 0, 0, 0, // id 2 at offset 4, one byte
        4, 0, 0, 0, b'b', b'o', b'o', b'l', // its tag
        0,    // bool
    ]);
    assert_eq!(bytes, expected);
}

#[test]
fn an_enum_kind_appends_its_size_and_values() {
    let mut bytes = Vec::new();
    GoldenEnum::SCHEMA.info().write_schema(&mut bytes);
    let mut expected = GoldenEnum::KEY.0.to_le_bytes().to_vec();
    // Version 1, a component, derived, untracked; size 1, alignment 1, one field.
    expected.extend_from_slice(&[1, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    // Id 7 at offset 0, one byte.
    expected.extend_from_slice(&[7, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
    let tag = <Two as Field>::NAME;
    expected.extend_from_slice(&(tag.len() as u32).to_le_bytes());
    expected.extend_from_slice(tag.as_bytes());
    // Enum, one byte wide, two variants: 0 and 3.
    expected.extend_from_slice(&[7, 1, 2, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(bytes, expected);
}
