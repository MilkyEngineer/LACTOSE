//! The types allowed as fields, and the kind each one reports.

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::{EntityRef, FieldId, NetId, PlainArray, PlayerId, Tick};

use crate::info::StructInfo;
use crate::visit::{FieldVisitor, ReferenceVisitor};

/// What a field holds, as far as generic code needs to know.
#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    /// `bool`.
    Bool,
    /// An unsigned integer or an identity newtype over one. The field size gives the width.
    Unsigned,
    /// A signed integer. The field size gives the width.
    Signed,
    /// A raw `f32` or `f64`. Presentation data only: inside the determinism boundary floats
    /// are [`FieldKind::Scalar`].
    Float,
    /// The simulation scalar of `lactose-math`, or a type built from it.
    Scalar,
    /// An [`EntityRef`], remapped on load.
    EntityRef,
    /// The asset reference of `lactose-asset-api`, remapped on load.
    AssetRef,
    /// A fieldless enum declared with `#[value]`.
    Enum(&'static EnumInfo),
    /// A nested struct declared with `#[value]`.
    Struct(&'static StructInfo),
    /// A fixed array of `len` elements.
    Array {
        /// The kind of each element.
        element: &'static FieldKind,
        /// The number of elements.
        len: usize,
    },
    /// Explicit padding: always zero, never visited, hashed or saved.
    Padding,
}

/// One variant of a `#[value]` enum.
#[derive(Clone, Copy, Debug)]
pub struct VariantInfo {
    /// The variant's Rust name.
    pub name: &'static str,
    /// The variant's explicit discriminant.
    pub value: u32,
}

/// The description of a `#[value]` enum.
#[derive(Clone, Copy, Debug)]
pub struct EnumInfo {
    /// The enum's Rust name.
    pub name: &'static str,
    /// Size in bytes: one, two or four.
    pub size: usize,
    /// The variants in declaration order.
    pub variants: &'static [VariantInfo],
}

/// A type allowed as a field of a component, a resource or a `#[value]` struct.
///
/// Implemented here for the integers up to 64 bits, `bool`, `f32` and `f64`, the identity
/// types of `lactose-core`, and [`PlainArray`] of any field type. `#[value]` implements it
/// for nested structs and fieldless enums. A crate that owns a plain-data type implements it
/// by forwarding to the visitor method that matches the type's kind.
///
/// A bare `[T; N]` is deliberately not a field: `bytemuck` treats it as plain data only when
/// `T` accepts every bit pattern, so an array of `bool`, enums or references would fail with
/// errors from its derives. [`PlainArray`] is the one spelling that works for every element.
/// `usize`, `isize`, `char` and the 128-bit integers are absent too: their size, validity or
/// alignment is not something a save format should depend on.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be a reflected field",
    note = "fields are integers up to 64 bits, `bool`, floats, `lactose_core` identity types, `EntityRef` and `#[value]` types",
    note = "an array field is a `lactose_core::PlainArray<T, N>`, never a bare `[T; N]`"
)]
pub trait Field: NoUninit + CheckedBitPattern + Send + Sync + 'static {
    /// The kind metadata reports for this type.
    const KIND: FieldKind;

    /// Reports this value to `visitor` under field `id`.
    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V);

    /// Reports every reference inside this value to `visitor` under field `id`.
    fn visit_references<V: ReferenceVisitor>(&mut self, id: FieldId, visitor: &mut V);
}

macro_rules! bytes_fields {
    ($($kind:ident: $($ty:ty),+;)+) => {$($(
        impl Field for $ty {
            const KIND: FieldKind = FieldKind::$kind;

            fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
                visitor.bytes(id, bytemuck::bytes_of(self));
            }

            fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
        }
    )+)+};
}

bytes_fields! {
    Bool: bool;
    Unsigned: u8, u16, u32, u64, Tick, PlayerId, NetId, Option<NetId>;
    Signed: i8, i16, i32, i64;
}

impl Field for f32 {
    const KIND: FieldKind = FieldKind::Float;

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.f32(id, *self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl Field for f64 {
    const KIND: FieldKind = FieldKind::Float;

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.f64(id, *self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl Field for EntityRef {
    const KIND: FieldKind = FieldKind::EntityRef;

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.entity_ref(id, *self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, id: FieldId, visitor: &mut V) {
        visitor.entity_ref(id, self);
    }
}

impl<T: Field, const N: usize> Field for PlainArray<T, N> {
    const KIND: FieldKind = FieldKind::Array { element: &T::KIND, len: N };

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        for element in &self.0 {
            element.visit(id, visitor);
        }
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, id: FieldId, visitor: &mut V) {
        for element in &mut self.0 {
            element.visit_references(id, visitor);
        }
    }
}

/// A byte whose only value is zero: the element of [`Padding`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, NoUninit, CheckedBitPattern)]
#[repr(u8)]
pub enum ZeroByte {
    /// Zero.
    #[default]
    Zero = 0,
}

/// `N` bytes of explicit padding, the type of a `#[field(padding)]` field.
///
/// It has no value other than all zeroes, so padding is zero in every live component and a
/// non-zero padding byte arriving from disk or the network fails validation. Write it as
/// `Padding::default()`.
pub type Padding<const N: usize> = PlainArray<ZeroByte, N>;

mod sealed {
    pub trait Sealed {}
    impl<const N: usize> Sealed for super::Padding<N> {}
}

/// Implemented by [`Padding`] alone, so nothing else can be declared as padding.
#[diagnostic::on_unimplemented(
    message = "a `#[field(padding)]` field must have the type `lactose_reflect::Padding<N>`",
    label = "`{Self}` is not `Padding<N>`"
)]
pub trait PaddingField: sealed::Sealed + NoUninit + CheckedBitPattern {}

impl<const N: usize> PaddingField for Padding<N> {}
