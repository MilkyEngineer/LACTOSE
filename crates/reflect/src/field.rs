//! The types allowed as fields, and the kind and type tag each one reports.

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::{EntityRef, FieldId, NetId, PlainArray, PlayerId, Tick};

use crate::info::StructInfo;
use crate::visit::{FieldVisitor, ReferenceVisitor};

/// What a field holds, as far as generic code needs to know.
///
/// A kind is a statement by the field type's [`Field`] impl. It selects nothing about
/// validity, which always comes from the type's own `CheckedBitPattern`; registration checks
/// it against the field's size.
#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    /// `bool`.
    Bool,
    /// An unsigned integer or an identity newtype over one. The field size gives the width.
    Unsigned,
    /// A signed integer. The field size gives the width.
    Signed,
    /// A raw `f32` or `f64`. Presentation data only; the world hasher refuses it.
    Float,
    /// A single scalar whose type implements [`ScalarField`]. Built by [`FieldKind::scalar`]
    /// only, so the kind means "some crate implemented `ScalarField` for this type" and no
    /// more; see that trait. Vectors and other types made of scalars are
    /// [`FieldKind::Struct`].
    #[non_exhaustive]
    Scalar,
    /// An [`EntityRef`], remapped on load. Generic code reads and rewrites a field of this
    /// kind as an `EntityRef` (the ECS's erased writes and reference walkers), so only
    /// `EntityRef` itself may declare it.
    EntityRef,
    /// A reference whose type implements [`AssetReference`], remapped on load. Built by
    /// [`FieldKind::asset_ref`] only, so the kind means "some crate implemented
    /// `AssetReference` for this type" and no more.
    #[non_exhaustive]
    AssetRef,
    /// A fieldless enum declared with `#[value]`.
    Enum(&'static EnumInfo),
    /// A nested struct declared with `#[value]`.
    Struct(&'static StructInfo),
    /// A fixed array of `len` elements.
    Array {
        /// The kind of each element.
        element: &'static FieldKind,
        /// The type tag of each element, its [`Field::NAME`].
        element_name: &'static str,
        /// The number of elements.
        len: usize,
    },
    /// Explicit padding: always zero, never visited, hashed or saved.
    Padding,
}

impl FieldKind {
    /// The kind of a field of type `S`, a scalar.
    #[must_use]
    pub const fn scalar<S: ScalarField>() -> Self {
        Self::Scalar
    }

    /// The kind of a field of type `A`, an asset reference.
    #[must_use]
    pub const fn asset_ref<A: AssetReference>() -> Self {
        Self::AssetRef
    }
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
/// This is a safe trait and an impl cannot make validation wrong: the size, alignment and
/// validity of a field are always taken from the type's `NoUninit` and `CheckedBitPattern`
/// impls, never from [`Field::KIND`] or [`Field::NAME`]. A wrong kind is caught at
/// registration where the kind fixes a size, and otherwise only mislabels the schema.
///
/// A bare `[T; N]` is deliberately not a field: `bytemuck` treats it as plain data only when
/// `T` accepts every bit pattern, so an array of `bool`, enums or references would fail with
/// errors from its derives. [`PlainArray`] is the one spelling that works for every element.
/// `usize`, `isize`, `char` and the 128-bit integers are absent too: their size, validity or
/// alignment is not something a save format should depend on.
///
/// The attributes of this crate write the `CheckedBitPattern` impl of a struct themselves.
/// A crate that instead applies `bytemuck`'s derive to a struct should know two things about
/// it: it generates a `pub struct <Name>Bits` beside the type, so that name is taken, and it
/// formats the `Bits` of every field, so a field type with a hand-written `CheckedBitPattern`
/// needs a `Bits` type that is `Debug`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be a reflected field",
    note = "fields are integers up to 64 bits, `bool`, floats, `lactose_core` identity types, `EntityRef` and `#[value]` types",
    note = "an array field is a `lactose_core::PlainArray<T, N>`, never a bare `[T; N]`"
)]
pub trait Field: NoUninit + CheckedBitPattern + Send + Sync + 'static {
    /// The kind metadata reports for this type.
    const KIND: FieldKind;

    /// The stable type tag the schema dump prints, such as `"u32"` or `"NetId"`. Two types
    /// with different meanings or encodings have different tags, and a tag never changes:
    /// replacing a field's type by one with another tag is a schema change.
    const NAME: &'static str;

    /// Reports this value to `visitor` under field `id`.
    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V);

    /// Reports every reference inside this value to `visitor` under field `id`.
    fn visit_references<V: ReferenceVisitor>(&mut self, id: FieldId, visitor: &mut V);
}

/// A scalar that states its own canonical bits. `lactose-math` implements it for the
/// simulation scalar, whose `Field` impl reports [`FieldKind::scalar`] and calls
/// [`FieldVisitor::scalar`].
///
/// This is a safe trait that any crate can implement, so neither it nor
/// [`FieldKind::Scalar`] proves that a value is the engine's scalar or that its bits are
/// canonical. What it provides is a reviewable fact: each impl is one place where a crate
/// claims the property below. The world hasher in `lactose-sim` therefore decides which
/// scalar types it accepts by naming them (a bound on the `lactose-math` type, or an
/// allowlist of impls checked in review), never by trusting the kind or this trait.
pub trait ScalarField: Field {
    /// The value's canonical bit pattern, zero-extended: equal values give equal bits on
    /// every platform, with one pattern for all NaNs.
    fn canonical_bits(&self) -> u64;
}

/// A typed reference to an asset. `lactose-asset-api` implements it; its `Field` impl
/// reports [`FieldKind::asset_ref`] and calls the `asset_ref` method of both visitors.
///
/// Like [`ScalarField`] this is a safe trait any crate can implement: the kind says that a
/// type claims to be an asset reference, not that it is the engine's.
///
/// Like [`ScalarField`] this is a safe trait any crate can implement: the kind says that a
/// type claims to be an asset reference, not that it is the engine's.
pub trait AssetReference: Field {
    /// The raw 64-bit asset identity, or zero for the empty reference.
    fn asset_id(&self) -> u64;

    /// Points this reference at the asset with raw identity `id`; zero empties it.
    fn set_asset_id(&mut self, id: u64);
}

macro_rules! bytes_fields {
    ($($kind:ident: $($ty:ty => $name:literal),+;)+) => {$($(
        impl Field for $ty {
            const KIND: FieldKind = FieldKind::$kind;
            const NAME: &'static str = $name;

            fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
                visitor.bytes(id, bytemuck::bytes_of(self));
            }

            fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
        }
    )+)+};
}

bytes_fields! {
    Bool: bool => "bool";
    Unsigned:
        u8 => "u8", u16 => "u16", u32 => "u32", u64 => "u64", Tick => "Tick",
        PlayerId => "PlayerId", NetId => "NetId", Option<NetId> => "Option<NetId>";
    Signed: i8 => "i8", i16 => "i16", i32 => "i32", i64 => "i64";
}

impl Field for f32 {
    const KIND: FieldKind = FieldKind::Float;
    const NAME: &'static str = "f32";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.f32(id, *self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl Field for f64 {
    const KIND: FieldKind = FieldKind::Float;
    const NAME: &'static str = "f64";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.f64(id, *self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

impl Field for EntityRef {
    const KIND: FieldKind = FieldKind::EntityRef;
    const NAME: &'static str = "EntityRef";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.entity_ref(id, *self);
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, id: FieldId, visitor: &mut V) {
        visitor.entity_ref(id, self);
    }
}

impl<T: Field, const N: usize> Field for PlainArray<T, N> {
    const KIND: FieldKind = FieldKind::Array { element: &T::KIND, element_name: T::NAME, len: N };
    // The dump renders an array from its kind, as `[<element>; N]`.
    const NAME: &'static str = "PlainArray";

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
