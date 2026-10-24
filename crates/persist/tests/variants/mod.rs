//! One base component and a variant of it per schema change, each in its own module under
//! the same key: shared by the schema tests (what each change does to the hash and the
//! digest) and the migration tests (what a load under each variant refuses).

#![allow(dead_code, reason = "each test file uses part of the variants")]

use lactose_core::PlainArray;
use lactose_math::Scalar;
use lactose_reflect::{Padding, component, resource, value};

/// The base type's module, with the parts each case changes passed in whole: the
/// component's attribute, the four value types and the fields.
macro_rules! base {
    ($module:ident, [$($attribute:tt)*], $kind:item, $elem:item, $inner:item, $inner_arr:item, { $($fields:tt)* }) => {
        pub mod $module {
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
pub mod resource_role {
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
