//! The schema hash and the layout digest (README, "Schemas"): golden values for a small
//! registry; what changes each and what changes neither, one change per case against one
//! base type; and the types `from_registry` refuses.

#![forbid(unsafe_code)]

use lactose_core::{ErrorKind, FieldId, PlainArray, xxh64};
use lactose_math::Scalar;
use lactose_persist::{LayoutDigest, SchemaHash, SchemaTable};
use lactose_reflect::{
    Field, FieldKind, FieldVisitor, Padding, ReferenceVisitor, Reflect, Registry, ScalarField,
    TypeInfo, component, resource, value,
};

/// The base type's module, with the parts each case changes passed in whole: the
/// component's attribute, the four value types and the fields.
macro_rules! base {
    ($module:ident, [$($attribute:tt)*], $kind:item, $elem:item, $inner:item, $inner_arr:item, { $($fields:tt)* }) => {
        mod $module {
            #![allow(dead_code, reason = "a type is described, never built")]
            use super::*;
            $kind
            $elem
            $inner
            $inner_arr
            #[component($($attribute)*)]
            pub struct Base { $($fields)* }
        }
    };
}

macro_rules! kind {
    () => {
        #[value]
        #[repr(u8)]
        pub enum Kind {
            A = 0,
            B = 1,
        }
    };
}

macro_rules! elem {
    () => {
        #[value]
        #[repr(u8)]
        pub enum Elem {
            A = 0,
            B = 1,
        }
    };
}

macro_rules! inner {
    () => {
        #[value]
        pub struct Inner {
            #[field(id = 1)]
            pub a: u32,
            #[field(id = 2)]
            pub b: u32,
        }
    };
}

macro_rules! inner_arr {
    () => {
        #[value]
        pub struct InnerArr {
            #[field(id = 1)]
            pub a: u32,
            #[field(id = 2)]
            pub b: u32,
        }
    };
}

base!(original, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

// ---- Changes neither sees ----

base!(replicated, [key = "schema::base", persistent, replicated], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(field_renamed, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub total: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(struct_renamed, [key = "schema::base", persistent], kind!{}, elem!{},
    #[value]
    pub struct Nested {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    },
    inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Nested,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(enum_renamed, [key = "schema::base", persistent],
    #[value]
    #[repr(u8)]
    pub enum Sort {
        A = 0,
        B = 1,
    },
    elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Sort,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

// ---- Changes the hash sees and the digest does not ----

base!(bumped, [key = "schema::base", persistent, version = 2], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(transient, [key = "schema::base", transient], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

/// The base type as a resource: a role change.
mod resource_role {
    #![allow(dead_code, reason = "a type is described, never built")]
    use super::*;
    kind! {}
    elem! {}
    inner! {}
    inner_arr! {}
    #[resource(key = "schema::base", persistent)]
    pub struct Base {
        #[field(id = 1)]
        pub count: u64,
        #[field(id = 2)]
        pub inner: Inner,
        #[field(id = 6)]
        pub values: PlainArray<u32, 2>,
        #[field(id = 7)]
        pub inners: PlainArray<InnerArr, 2>,
        #[field(id = 3)]
        pub kind: Kind,
        #[field(id = 4)]
        pub kinds: PlainArray<Elem, 2>,
        #[field(id = 5)]
        pub small: u8,
        #[field(padding)]
        _pad: Padding<4>,
    }
}

// ---- Layout changes the document schema sees ----

base!(field_id, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 8)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(field_offset, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(padding)] _pad: Padding<4>,
});

base!(field_size, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 3>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
});

base!(type_size, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<12>,
});

// ---- Layout changes only the digest sees ----

base!(nested_reorder, [key = "schema::base", persistent], kind!{}, elem!{},
    #[value]
    pub struct Inner {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    },
    inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(renumbered, [key = "schema::base", persistent],
    #[value]
    #[repr(u8)]
    pub enum Kind {
        A = 0,
        B = 2,
    },
    elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(names_swapped, [key = "schema::base", persistent],
    #[value]
    #[repr(u8)]
    pub enum Kind {
        B = 0,
        A = 1,
    },
    elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(element_names_swapped, [key = "schema::base", persistent], kind!{},
    #[value]
    #[repr(u8)]
    pub enum Elem {
        B = 0,
        A = 1,
    },
    inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(declarations_reordered, [key = "schema::base", persistent],
    #[value]
    #[repr(u8)]
    pub enum Kind {
        B = 1,
        A = 0,
    },
    elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(variant_renamed, [key = "schema::base", persistent],
    #[value]
    #[repr(u8)]
    pub enum Kind {
        A = 0,
        C = 1,
    },
    elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(signed, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: i64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(scalar, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: Scalar,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(element_signed, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<i32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(element_reorder, [key = "schema::base", persistent], kind!{}, elem!{}, inner!{},
    #[value]
    pub struct InnerArr {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    },
    {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

base!(element_enum, [key = "schema::base", persistent], kind!{},
    #[value]
    #[repr(u8)]
    pub enum Elem {
        A = 0,
        B = 2,
    },
    inner!{}, inner_arr!{}, {
    #[field(id = 1)] pub count: u64,
    #[field(id = 2)] pub inner: Inner,
    #[field(id = 6)] pub values: PlainArray<u32, 2>,
    #[field(id = 7)] pub inners: PlainArray<InnerArr, 2>,
    #[field(id = 3)] pub kind: Kind,
    #[field(id = 4)] pub kinds: PlainArray<Elem, 2>,
    #[field(id = 5)] pub small: u8,
    #[field(padding)] _pad: Padding<4>,
});

// ---- Other types of the small registry ----

/// A transient resource.
#[resource(key = "schema::pulse", transient)]
pub struct Pulse(#[field(id = 1)] pub u32);

/// A derived component: never saved, never hashed.
#[component(key = "schema::cache", derived)]
pub struct Cache(#[field(id = 1)] pub u64);

/// A derived component with a raw float: derived types are not checked.
#[component(key = "schema::derived_float", derived)]
pub struct DerivedFloat(#[field(id = 1)] pub f32);

fn info<T: Reflect>() -> &'static TypeInfo {
    let mut registry = Registry::new();
    registry.register::<T>().unwrap();
    registry.get(T::KEY).unwrap()
}

fn digest<T: Reflect>() -> LayoutDigest {
    LayoutDigest::of(info::<T>())
}

/// The hash of a registry holding the base type `T` and the small registry's others.
fn hash_with<T: Reflect>() -> SchemaHash {
    let mut registry = Registry::new();
    registry.register::<T>().unwrap();
    registry.register::<Pulse>().unwrap();
    SchemaHash::of(&registry)
}

/// The document schema of `T`, as the table derives it.
fn document_of<T: Reflect>() -> lactose_bundle::ComponentSchema {
    let mut registry = Registry::new();
    registry.register::<T>().unwrap();
    SchemaTable::from_registry(&registry).unwrap().schema(T::KEY).unwrap().clone()
}

#[test]
fn golden_digests_and_hash() {
    // Pinned: a change to the digest's or the hash's encoding fails here, and every save
    // written before it would then be refused or misread.
    assert_eq!(digest::<original::Base>(), LayoutDigest(0x2fcc_778a_9874_9234));
    assert_eq!(digest::<Pulse>(), LayoutDigest(0x1722_9a34_206c_c97c));
    assert_eq!(digest::<Tiny>(), LayoutDigest(0x2226_b154_bd2d_f34f));
    assert_eq!(hash_with::<original::Base>(), SchemaHash(0x803f_9d0b_5860_ce66));
    // A type of primitives alone is digested as `write_schema` writes its layout.
    let pulse = info::<Pulse>();
    let mut schema = Vec::new();
    pulse.write_schema(&mut schema);
    assert_eq!(digest::<Pulse>(), LayoutDigest(xxh64(&schema[15..], 0)));
}

/// Two variants.
#[value]
#[repr(u8)]
enum Two {
    A = 0,
    B = 1,
}

/// A nested struct with padding.
#[value]
struct Pair {
    #[field(id = 1)]
    pub a: u16,
    #[field(id = 2)]
    pub b: u8,
    #[field(padding)]
    _pad: Padding<1>,
}

/// An enum and a nested struct: the two places the digest departs from `write_schema`.
#[component(key = "schema::tiny", persistent)]
struct Tiny {
    #[field(id = 1)]
    pub two: Two,
    #[field(padding)]
    _pad: Padding<1>,
    #[field(id = 2)]
    pub pair: Pair,
}

#[test]
fn the_digest_is_its_stream() {
    let mut stream = Vec::new();
    let u32s = |stream: &mut Vec<u8>, values: &[u32]| {
        values.iter().for_each(|value| stream.extend_from_slice(&value.to_le_bytes()));
    };
    let field = |stream: &mut Vec<u8>, id: u16, offset: u32, size: u32, tag: &str| {
        stream.extend_from_slice(&id.to_le_bytes());
        u32s(stream, &[offset, size, tag.len() as u32]);
        stream.extend_from_slice(tag.as_bytes());
    };
    // Size, alignment, two fields.
    u32s(&mut stream, &[6, 2, 2]);
    // The enum: an empty tag, kind 7, its size, two variants, each value and name.
    field(&mut stream, 1, 0, 1, "");
    stream.extend_from_slice(&[7, 1]);
    u32s(&mut stream, &[2, 0, 1]);
    stream.push(b'A');
    u32s(&mut stream, &[1, 1]);
    stream.push(b'B');
    // The struct: an empty tag, kind 8, its layout with its primitives' tags.
    field(&mut stream, 2, 2, 4, "");
    stream.push(8);
    u32s(&mut stream, &[4, 2, 2]);
    field(&mut stream, 1, 0, 2, "u16");
    stream.push(1);
    field(&mut stream, 2, 2, 1, "u8");
    stream.push(1);
    assert_eq!(digest::<Tiny>(), LayoutDigest(xxh64(&stream, 0)));
}

#[test]
fn the_hash_is_its_stream() {
    let base = info::<original::Base>();
    let pulse = info::<Pulse>();
    let mut stream = Vec::new();
    let mut types = [(base, 0u8), (pulse, 1u8)];
    types.sort_by_key(|(info, _)| info.key());
    for (info, role) in types {
        stream.extend_from_slice(&info.key().0.to_le_bytes());
        stream.extend_from_slice(&info.version().to_le_bytes());
        stream.push(role);
        stream.push(u8::from(info.key() == Pulse::KEY));
        stream.extend_from_slice(&LayoutDigest::of(info).0.to_le_bytes());
    }
    assert_eq!(hash_with::<original::Base>(), SchemaHash(xxh64(&stream, 0)));
}

#[test]
fn registration_order_derived_types_and_tracking_change_nothing() {
    let base = hash_with::<original::Base>();
    let mut reversed = Registry::new();
    reversed.register::<Pulse>().unwrap();
    reversed.register::<original::Base>().unwrap();
    assert_eq!(SchemaHash::of(&reversed), base);
    reversed.register::<Cache>().unwrap();
    reversed.register::<DerivedFloat>().unwrap();
    assert_eq!(SchemaHash::of(&reversed), base);
    assert_eq!(SchemaTable::from_registry(&reversed).unwrap().hash(), base);
    assert_eq!(hash_with::<replicated::Base>(), base);
    assert_eq!(digest::<replicated::Base>(), digest::<original::Base>());
}

#[test]
fn renames_change_nothing() {
    let (hash, layout) = (hash_with::<original::Base>(), digest::<original::Base>());
    assert_eq!(
        (hash_with::<field_renamed::Base>(), digest::<field_renamed::Base>()),
        (hash, layout)
    );
    assert_eq!(
        (hash_with::<struct_renamed::Base>(), digest::<struct_renamed::Base>()),
        (hash, layout)
    );
    assert_eq!((hash_with::<enum_renamed::Base>(), digest::<enum_renamed::Base>()), (hash, layout));
}

#[test]
fn version_role_and_persistence_change_the_hash_and_not_the_digest() {
    let (hash, layout) = (hash_with::<original::Base>(), digest::<original::Base>());
    for (what, changed, changed_layout) in [
        ("a version bump", hash_with::<bumped::Base>(), digest::<bumped::Base>()),
        ("a persistence change", hash_with::<transient::Base>(), digest::<transient::Base>()),
        ("a role change", hash_with::<resource_role::Base>(), digest::<resource_role::Base>()),
    ] {
        assert_ne!(changed, hash, "{what}");
        assert_eq!(changed_layout, layout, "{what}");
    }
}

#[test]
fn layout_changes_the_schema_sees_change_both() {
    let (hash, layout) = (hash_with::<original::Base>(), digest::<original::Base>());
    let document = document_of::<original::Base>();
    for (what, changed, changed_layout, changed_document) in [
        (
            "a field ID",
            hash_with::<field_id::Base>(),
            digest::<field_id::Base>(),
            document_of::<field_id::Base>(),
        ),
        (
            "a field offset",
            hash_with::<field_offset::Base>(),
            digest::<field_offset::Base>(),
            document_of::<field_offset::Base>(),
        ),
        (
            "a field size",
            hash_with::<field_size::Base>(),
            digest::<field_size::Base>(),
            document_of::<field_size::Base>(),
        ),
        (
            "a type's size",
            hash_with::<type_size::Base>(),
            digest::<type_size::Base>(),
            document_of::<type_size::Base>(),
        ),
    ] {
        assert_ne!(changed, hash, "{what}");
        assert_ne!(changed_layout, layout, "{what}");
        assert_ne!(changed_document, document, "{what}");
    }
}

#[test]
fn layout_changes_only_the_digest_sees() {
    let (hash, layout) = (hash_with::<original::Base>(), digest::<original::Base>());
    let document = document_of::<original::Base>();
    let cases = [
        (
            "a nested reorder",
            hash_with::<nested_reorder::Base>(),
            digest::<nested_reorder::Base>(),
            document_of::<nested_reorder::Base>(),
        ),
        (
            "discriminants renumbered",
            hash_with::<renumbered::Base>(),
            digest::<renumbered::Base>(),
            document_of::<renumbered::Base>(),
        ),
        (
            "names swapped over their values",
            hash_with::<names_swapped::Base>(),
            digest::<names_swapped::Base>(),
            document_of::<names_swapped::Base>(),
        ),
        (
            "an element's names swapped",
            hash_with::<element_names_swapped::Base>(),
            digest::<element_names_swapped::Base>(),
            document_of::<element_names_swapped::Base>(),
        ),
        (
            "declarations reordered",
            hash_with::<declarations_reordered::Base>(),
            digest::<declarations_reordered::Base>(),
            document_of::<declarations_reordered::Base>(),
        ),
        (
            "a variant renamed",
            hash_with::<variant_renamed::Base>(),
            digest::<variant_renamed::Base>(),
            document_of::<variant_renamed::Base>(),
        ),
        (
            "u64 to i64",
            hash_with::<signed::Base>(),
            digest::<signed::Base>(),
            document_of::<signed::Base>(),
        ),
        (
            "u64 to Scalar",
            hash_with::<scalar::Base>(),
            digest::<scalar::Base>(),
            document_of::<scalar::Base>(),
        ),
        (
            "an element's primitive",
            hash_with::<element_signed::Base>(),
            digest::<element_signed::Base>(),
            document_of::<element_signed::Base>(),
        ),
        (
            "an element's nested order",
            hash_with::<element_reorder::Base>(),
            digest::<element_reorder::Base>(),
            document_of::<element_reorder::Base>(),
        ),
        (
            "an element's enum",
            hash_with::<element_enum::Base>(),
            digest::<element_enum::Base>(),
            document_of::<element_enum::Base>(),
        ),
    ];
    let mut digests = vec![layout];
    for (what, changed, changed_layout, changed_document) in cases {
        assert_ne!(changed, hash, "{what}");
        assert_ne!(changed_layout, layout, "{what}");
        // A 32-bit scalar build changes the size too; the digest sees it either way.
        if what != "u64 to Scalar" || Scalar::BYTES == 8 {
            assert_eq!(changed_document, document, "{what}: the document schema is blind to it");
        }
        digests.push(changed_layout);
    }
    // No two cases collide.
    let count = digests.len();
    digests.sort_by_key(|digest| digest.0);
    digests.dedup();
    assert_eq!(digests.len(), count);
}

// ---- What `from_registry` refuses ----

/// A persistent component with a raw float.
#[component(key = "schema::raw_float", persistent)]
pub struct RawFloat(#[field(id = 1)] pub f32);

/// A float nested in a struct in an array.
#[value]
pub struct Floaty {
    /// The float.
    #[field(id = 1)]
    pub value: f64,
}

/// A transient component with a float nested in an array.
#[component(key = "schema::nested_float", transient)]
pub struct NestedFloat(#[field(id = 1)] pub PlainArray<Floaty, 2>);

/// A scalar that is not `lactose_math::Scalar`.
#[derive(Clone, Copy, PartialEq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(transparent)]
pub struct Meters(f64);

impl ScalarField for Meters {
    fn canonical_bits(&self) -> u64 {
        self.0.to_bits()
    }
}

impl Field for Meters {
    const KIND: FieldKind = FieldKind::scalar::<Self>();
    const NAME: &'static str = "Meters";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.scalar(id, self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

/// A persistent component with a foreign scalar.
#[component(key = "schema::foreign", persistent)]
pub struct Foreign(#[field(id = 1)] pub Meters);

/// A persistent resource over the 16 KiB a document carries per value.
#[resource(key = "schema::big", persistent)]
pub struct Big(#[field(id = 1)] pub PlainArray<u64, 2560>);

/// The same size, derived: never saved, so never refused.
#[resource(key = "schema::big_derived", derived)]
pub struct BigDerived(#[field(id = 1)] pub PlainArray<u64, 2560>);

fn table_of<T: Reflect>() -> lactose_core::Result<SchemaTable> {
    let mut registry = Registry::new();
    registry.register::<Pulse>().unwrap();
    registry.register::<T>().unwrap();
    SchemaTable::from_registry(&registry)
}

#[test]
fn from_registry_refuses_what_a_save_cannot_hold() {
    assert_eq!(table_of::<RawFloat>().unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(table_of::<NestedFloat>().unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(table_of::<Foreign>().unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(size_of::<Big>(), 20 * 1024);
    assert_eq!(table_of::<Big>().unwrap_err().kind(), ErrorKind::InvalidData);
    // Derived types are not saved, so neither is checked.
    table_of::<DerivedFloat>().unwrap();
    table_of::<BigDerived>().unwrap();
}

#[test]
fn the_table_knows_derived_types_and_gives_them_no_schema() {
    let mut registry = Registry::new();
    registry.register::<original::Base>().unwrap();
    registry.register::<Pulse>().unwrap();
    registry.register::<Cache>().unwrap();
    let table = SchemaTable::from_registry(&registry).unwrap();
    assert_eq!(table.len(), 2);
    assert!(table.schema(Cache::KEY).is_none());
    assert!(table.layout(Cache::KEY).is_none());
    assert_eq!(table.persistence(Cache::KEY), Some(lactose_reflect::Persistence::Derived));
    assert!(table.info(Cache::KEY).is_some());
    assert_eq!(table.layout(Pulse::KEY), Some(digest::<Pulse>()));
    let keys: Vec<_> = table.schemas().map(|schema| schema.component()).collect();
    let mut expected = vec![original::Base::KEY, Pulse::KEY];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}
