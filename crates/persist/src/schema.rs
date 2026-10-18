//! The persistence policy as this crate reads it from the registry (architecture sections
//! 15.1 and 15.4): which types a save carries, the [`ComponentSchema`] each document must
//! carry for them, the layout digest that tells apart what the schema cannot, and the
//! schema hash a container records.

use std::collections::BTreeMap;

use lactose_bundle::ComponentSchema;
use lactose_core::{ComponentKey, Result};
use lactose_reflect::{Persistence, Registry, TypeInfo, TypeRole};

/// The digest of a type's layout at one schema version: what a document's
/// [`ComponentSchema`] does not see, in one number.
///
/// It is the xxh64, seed zero, over the layout part of
/// [`TypeInfo::write_schema`](lactose_reflect::TypeInfo::write_schema): the bytes that
/// encoding appends after its first fifteen (the key, the version, the role, the
/// persistence and the tracking bits), that is, from the layout's size on: size,
/// alignment, the count of non-padding fields, and each field's ID, offset, size, type tag
/// and kind, with nested layouts, enum sizes and discriminants, and array lengths and
/// element kinds written out. The tracking bits are left out on purpose: marking a type
/// replicated or extracted changes no saved byte.
///
/// A `ComponentSchema` holds the key, the version, the size and each top-level field's ID,
/// offset and size, so three kinds of change leave it equal: a reorder inside a nested
/// `#[value]` struct, renumbered enum discriminants, and a same-size type change (`u64` to
/// `Scalar`, `i32` to `u32`). The digest sees all three, so a save carries it beside each
/// saved type's schema and [`Migrations::migrate`](crate::Migrations::migrate) refuses
/// equal versions with different digests. What no digest sees is a change of meaning
/// under an unchanged layout (a field's unit, a renamed variant with the same
/// discriminant): only a version bump and a step carry that, and the build-time guard
/// D129 names (a committed registry dump compared by a test) is what makes the bump
/// hard to forget.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LayoutDigest(pub u64);

impl LayoutDigest {
    /// The digest of `info`'s layout, as stated on the type.
    #[must_use]
    pub fn of(info: &TypeInfo) -> Self {
        todo!("hash the layout part of `write_schema`")
    }
}

/// The hash of every saved type's schema: what a container records in its header
/// (architecture section 15.4) and what decides, in one comparison, whether every document
/// a container holds matches the loading build's registry.
///
/// It is the xxh64, seed zero, over one stream holding, for every registered type whose
/// persistence is `Persistent` or `Transient`, components and resources alike, in
/// ascending [`ComponentKey`] order: the key as a `u64`, the version as a `u32`, the role
/// as a `u8` (0 component, 1 resource), the persistence as a `u8` (0 persistent, 1
/// transient) and the [`LayoutDigest`] as a `u64`, every integer little-endian. A
/// `derived` type is never saved and never counted, and the tracking bits are not in it,
/// so registering a derived type on one build only, or marking a type replicated, changes
/// nothing; a version, a layout (through the digest), a role or a persistence change does.
/// Two registries with equal hashes hold equal saved schemas; the hash is a shortcut, never
/// a proof a loader relies on: every section still carries its own schemas and digests,
/// and the loader compares per type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SchemaHash(pub u64);

impl SchemaHash {
    /// The hash of `registry`'s saved types, as stated on the type.
    #[must_use]
    pub fn of(registry: &Registry) -> Self {
        todo!("hash each saved type's key, version, role, persistence and layout digest")
    }
}

/// What the table keeps for a saved type.
#[derive(Clone, Debug)]
struct Saved {
    schema: ComponentSchema,
    layout: LayoutDigest,
}

/// One registered type as the table keeps it.
#[derive(Clone, Debug)]
struct Entry {
    info: &'static TypeInfo,
    /// `None` for a `derived` type, which no document may carry.
    saved: Option<Saved>,
}

/// Every type a save may carry, with the schema each document carries for it and its
/// layout digest, and the persistence of every registered type, so the diff can tell a
/// transient column from a persistent one and a derived column from an unregistered one.
///
/// Built once per registry by [`SchemaTable::from_registry`] and shared: the diff of every
/// region, the save and the loader all read it. It holds what the registry's `'static`
/// descriptions state and nothing else, so it is a pure function of the registered types,
/// the same on every machine and in every registration order (the registry is keyed).
///
/// The schema of a type is derived exactly as the ECS loader derives the one it compares a
/// document's against (`World::load_region` refuses a document whose schema differs with
/// `VersionMismatch`): the component key, the schema version, the layout's size, and one
/// [`FieldLayout`](lactose_bundle::FieldLayout) per non-padding top-level field with its ID,
/// offset and size, in field-ID order. Padding fields and nested structure are not in it;
/// a nested `#[value]` struct is one field of its outer size. What it does not see, the
/// [`LayoutDigest`] does.
#[derive(Clone, Debug)]
pub struct SchemaTable {
    entries: BTreeMap<ComponentKey, Entry>,
    hash: SchemaHash,
}

impl SchemaTable {
    /// The table of `registry`. Fails with `Unsupported` if a saved type's layout holds,
    /// at any depth, a raw `f32` or `f64` (`FieldKind::Float`: its NaN sign and payload
    /// are not canonical, so it is never saved) or a `FieldKind::Scalar` field whose type
    /// tag is not `lactose_math::Scalar`'s `Field::NAME` (the one scalar this crate
    /// canonicalises); `lactose-sim` refuses the same types at registration, so neither
    /// reaches a world. Fails with `InvalidData` if a saved type is larger than
    /// `lactose_bundle::MAX_COMPONENT_BYTES` (16 KiB), the one size a document carries:
    /// the ECS caps components there and not resources, so a persistent region resource
    /// over it is legal to the world and refused here, for the whole table
    /// (`README.md`, "Policy"; D129 names the refusal at registration as the fix).
    pub fn from_registry(registry: &Registry) -> Result<Self> {
        todo!("derive one schema and digest per saved type and the table's hash")
    }

    /// The hash of the saved types' schemas, [`SchemaHash::of`] the same registry.
    #[must_use]
    pub fn hash(&self) -> SchemaHash {
        self.hash
    }

    /// The schema a document carries for `component`: `None` for a type that is not
    /// registered or is `derived`.
    #[must_use]
    pub fn schema(&self, component: ComponentKey) -> Option<&ComponentSchema> {
        Some(&self.entries.get(&component)?.saved.as_ref()?.schema)
    }

    /// The layout digest of `component`, [`LayoutDigest::of`] its description: `None` for
    /// a type that is not registered or is `derived`.
    #[must_use]
    pub fn layout(&self, component: ComponentKey) -> Option<LayoutDigest> {
        Some(self.entries.get(&component)?.saved.as_ref()?.layout)
    }

    /// The registry's description of `component`, for any registered type.
    #[must_use]
    pub fn info(&self, component: ComponentKey) -> Option<&'static TypeInfo> {
        Some(self.entries.get(&component)?.info)
    }

    /// The persistence of `component`, for any registered type.
    #[must_use]
    pub fn persistence(&self, component: ComponentKey) -> Option<Persistence> {
        Some(self.info(component)?.persistence())
    }

    /// Whether `component` is a registered component or resource.
    #[must_use]
    pub fn role(&self, component: ComponentKey) -> Option<TypeRole> {
        Some(self.info(component)?.role())
    }

    /// The schema of every saved type, strictly ascending by component key: what
    /// `Overlay::new` and `Baseline::from_rows` take as their schema argument.
    #[must_use]
    pub fn schemas(&self) -> Schemas<'_> {
        Schemas { entries: self.entries.values() }
    }

    /// The number of saved types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.values().filter(|entry| entry.saved.is_some()).count()
    }

    /// Whether no type is saved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The saved types' schemas, ascending by component key. See [`SchemaTable::schemas`].
#[derive(Clone, Debug)]
pub struct Schemas<'a> {
    entries: std::collections::btree_map::Values<'a, ComponentKey, Entry>,
}

impl<'a> Iterator for Schemas<'a> {
    type Item = &'a ComponentSchema;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.by_ref().find_map(|entry| entry.saved.as_ref().map(|saved| &saved.schema))
    }
}
