//! The persistence policy as this crate reads it from the registry (architecture sections
//! 15.1 and 15.4): which types a save carries, the [`ComponentSchema`] each document must
//! carry for them, and the schema hash a container records.

use std::collections::BTreeMap;

use lactose_bundle::ComponentSchema;
use lactose_core::{ComponentKey, Result};
use lactose_reflect::{Persistence, Registry, TypeInfo, TypeRole};

/// The hash of every saved type's schema: what a container records in its header
/// (architecture section 15.4) and what decides, in one comparison, whether every document
/// a container holds matches the loading build's registry.
///
/// It is the xxh64, seed zero, over the concatenation of
/// [`TypeInfo::write_schema`](lactose_reflect::TypeInfo::write_schema) of every registered
/// type whose persistence is `Persistent` or `Transient`, components and resources alike,
/// in ascending [`ComponentKey`] order. A `derived` type is never saved and never counted,
/// so registering one on one build only changes nothing. Two registries with equal hashes
/// hold equal saved schemas; the hash is a shortcut, never a proof a loader relies on: every
/// document still carries its own schema table, and the loader compares per component.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SchemaHash(pub u64);

impl SchemaHash {
    /// The hash of `registry`'s saved types, as stated on the type.
    #[must_use]
    pub fn of(registry: &Registry) -> Self {
        todo!("hash the saved types' `write_schema` bytes in ascending key order")
    }
}

/// One registered type as the table keeps it.
#[derive(Clone, Debug)]
struct Entry {
    info: &'static TypeInfo,
    /// `None` for a `derived` type, which no document may carry.
    schema: Option<ComponentSchema>,
}

/// Every type a save may carry, with the schema each document carries for it, and the
/// persistence of every registered type, so the diff can tell a transient column from a
/// persistent one and a derived column from an unregistered one.
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
/// a nested `#[value]` struct is one field of its outer size.
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
    /// `lactose_bundle::MAX_COMPONENT_BYTES`.
    pub fn from_registry(registry: &Registry) -> Result<Self> {
        todo!("derive one schema per saved type and the table's hash")
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
        self.entries.get(&component)?.schema.as_ref()
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
        self.entries.values().filter(|entry| entry.schema.is_some()).count()
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
        self.entries.by_ref().find_map(|entry| entry.schema.as_ref())
    }
}
