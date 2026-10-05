//! Reflection metadata for components and resources: stable keys, explicit field IDs, typed
//! reference fields, persistence policy, per-type field visitors and the type registry.
//!
//! Foundation tier, outside the determinism boundary. This crate supplies metadata and
//! statically dispatched visitors only; hashing, copying and remapping are visitors that
//! `lactose-sim` implements (architecture section 8.1). See `README.md`.

#![forbid(unsafe_code)]

mod field;
mod info;
mod registry;
mod visit;

pub use bytemuck;
pub use field::{
    AssetReference, EnumInfo, Field, FieldKind, Padding, PaddingField, ScalarField, VariantInfo,
    ZeroByte,
};
pub use info::{FieldInfo, Persistence, Schema, StructInfo, Tracking, TypeInfo, TypeRole, decode};
/// Declares a component: `#[component(key = "game::health", persistent)]` on a struct.
///
/// Arguments, in any order:
///
/// - `key = "crate::name"`, required: the stable name. Its xxh64 is the [`Reflect::KEY`].
/// - exactly one of `persistent`, `transient`, `derived`, required: the [`Persistence`].
/// - `version = N`, default 1: the schema version, raised when a migration is written.
/// - `replicated`, `extracted`: the [`Tracking`] flags.
/// - `retired = [N, ...]`: the IDs of fields that have been deleted. Deleting a field moves
///   its ID here, and no field may use a retired ID.
/// - `crate = "path"`: where the reflect crate is found, for a crate that re-exports these
///   attributes. The default is `::lactose_reflect`.
///
/// Every field carries `#[field(id = N)]` with an ID the type has never used before, or
/// `#[field(padding)]` if it is a [`Padding`]. Field types implement [`Field`].
///
/// The attribute lays the struct out as `#[repr(C)]` in declaration order and implements
/// `Clone`, `Copy`, `bytemuck::NoUninit` and `bytemuck::CheckedBitPattern`. It must be the
/// outermost attribute, above any `#[derive]`: a derive written above it runs first, on the
/// struct as written, and fails on the `#[field]` attributes or collides with the impls
/// added here. `Clone` and `Copy` may be listed in a derive below it. A struct with implicit
/// padding does not compile; the error names the field to move or the padding to insert.
/// Generic types, enums, structs with their own `#[repr]`, `#[cfg_attr]` on the type and
/// `#[cfg]` on a field are rejected, since each would let two builds disagree about one key.
///
/// ```
/// use lactose_reflect::{Padding, Reflect, component};
///
/// #[component(key = "doc::fuse", persistent, replicated)]
/// pub struct Fuse {
///     #[field(id = 1)]
///     pub ticks_left: u32,
///     #[field(id = 2)]
///     pub lit: bool,
///     #[field(padding)]
///     _pad: Padding<3>,
/// }
///
/// let fuse = Fuse { ticks_left: 180, lit: true, _pad: Padding::default() };
/// assert_eq!(Fuse::SCHEMA.layout().size(), size_of_val(&fuse));
/// ```
pub use lactose_reflect_macros::component;
/// Declares a reflected resource: `#[resource(key = "game::gravity", persistent)]` on a
/// struct. It accepts what [`macro@component`] accepts and differs only in implementing
/// [`Resource`] and reporting [`TypeRole::Resource`].
pub use lactose_reflect_macros::resource;
/// Declares a type usable as a field: a nested struct or a fieldless enum.
///
/// A struct follows the field rules of [`macro@component`]; its field IDs are its own, and
/// it accepts `retired = [N, ...]` and `crate = "path"`. An enum states its width with `#[repr(u8)]`, `#[repr(u16)]` or `#[repr(u32)]` and gives
/// every variant an explicit discriminant, because both are part of the save format.
/// Data-carrying enums are rejected.
///
/// ```
/// use lactose_reflect::{component, value};
///
/// #[value]
/// #[repr(u8)]
/// #[derive(PartialEq, Debug)]
/// pub enum Stance {
///     Standing = 0,
///     Climbing = 1,
/// }
///
/// #[component(key = "doc::posture", persistent)]
/// pub struct Posture(#[field(id = 1)] pub Stance);
/// ```
pub use lactose_reflect_macros::value;
pub use registry::Registry;
pub use visit::{FieldVisitor, ReferenceVisitor};

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::ComponentKey;

/// A registered plain-data type: a [`Component`] or a [`Resource`].
///
/// Implemented by `#[component(...)]` and `#[resource(...)]`, which is the only supported way
/// to implement it. The trait is safe, and a wrong hand-written impl cannot make
/// [`TypeInfo::validate`] or [`decode`] accept bytes that are invalid for the type, because
/// [`Schema<Self>`] takes size and validity from the type. It can misdescribe the fields;
/// see [`TypeInfo::write_field`] for what that affects.
pub trait Reflect: NoUninit + CheckedBitPattern + Send + Sync + 'static {
    /// The build, wire and disk key: xxh64 of the declared name.
    const KEY: ComponentKey;

    /// The description of this type the registry holds.
    const SCHEMA: &'static Schema<Self>;

    /// Calls `visitor` for every non-padding field, in declaration order.
    fn visit<V: FieldVisitor>(&self, visitor: &mut V);

    /// Calls `visitor` for every entity and asset reference in the value, in declaration
    /// order, so it can rewrite them in place.
    fn visit_references<V: ReferenceVisitor>(&mut self, visitor: &mut V);
}

/// A plain-data type stored one per entity, in a chunk column. Declared with
/// `#[component(key = "...", ...)]`.
pub trait Component: Reflect {}

/// A plain-data singleton with declared access. Declared with `#[resource(key = "...", ...)]`.
///
/// Service resources (`dyn` traits that `app` inserts) are not reflected and do not implement
/// this trait.
pub trait Resource: Reflect {}

/// Paths the macro expansion names. Not part of the interface.
#[doc(hidden)]
pub mod __private {
    pub use lactose_core::{ComponentKey, FieldId};
}
