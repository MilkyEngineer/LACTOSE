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
pub use field::{EnumInfo, Field, FieldKind, Padding, PaddingField, VariantInfo, ZeroByte};
pub use info::{FieldInfo, Persistence, StructInfo, Tracking, TypeInfo, TypeRole, decode};
/// Declares a component: `#[component(key = "game::health", persistent)]` on a struct.
///
/// Arguments, in any order:
///
/// - `key = "crate::name"`, required: the stable name. Its xxh64 is the [`Reflect::KEY`].
/// - exactly one of `persistent`, `transient`, `derived`, required: the [`Persistence`].
/// - `version = N`, default 1: the schema version, raised when a migration is written.
/// - `replicated`, `extracted`: the [`Tracking`] flags.
///
/// Every field carries `#[field(id = N)]` with an ID the type has never used before, or
/// `#[field(padding)]` if it is a [`Padding`]. Field types implement [`Field`].
///
/// The attribute lays the struct out as `#[repr(C)]` in declaration order and derives
/// `Clone`, `Copy`, `bytemuck::NoUninit` and `bytemuck::CheckedBitPattern`, so write it above
/// any `#[derive]` and do not derive those again. A struct with implicit padding does not
/// compile; the error names the field to move or the padding to insert. Generic types, enums
/// and structs with their own `#[repr]` are rejected.
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
/// assert_eq!(Fuse::INFO.layout().size(), size_of_val(&fuse));
/// ```
pub use lactose_reflect_macros::component;
/// Declares a reflected resource: `#[resource(key = "game::gravity", persistent)]` on a
/// struct. It accepts what [`macro@component`] accepts and differs only in implementing
/// [`Resource`] and reporting [`TypeRole::Resource`].
pub use lactose_reflect_macros::resource;
/// Declares a type usable as a field: a nested struct or a fieldless enum.
///
/// A struct follows the field rules of [`macro@component`]; its field IDs are its own. An
/// enum states its width with `#[repr(u8)]`, `#[repr(u16)]` or `#[repr(u32)]` and gives
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
/// to implement it. [`Registry::register`] re-checks everything stated here against the type.
pub trait Reflect: NoUninit + CheckedBitPattern + Send + Sync + 'static {
    /// The build, wire and disk key: xxh64 of the declared name.
    const KEY: ComponentKey;

    /// The description of this type the registry holds.
    const INFO: &'static TypeInfo;

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
