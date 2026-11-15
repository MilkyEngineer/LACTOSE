//! The persistence policy as this crate reads it from the registry (architecture sections
//! 15.1 and 15.4): which types a save carries, the [`ComponentSchema`] each document must
//! carry for them, digest included, and the schema hash a container records. The schema
//! and its [`LayoutDigest`] are `lactose-ecs`'s (`lactose_ecs::document_schema`), the one
//! derivation the ECS loader compares a document against; this crate re-exports the
//! digest's type.

use std::collections::BTreeMap;

use lactose_bundle::{ComponentSchema, MAX_COMPONENT_BYTES};
use lactose_core::{ComponentKey, Error, ErrorKind, Result, xxh64};
pub use lactose_ecs::LayoutDigest;
use lactose_ecs::document_schema;
use lactose_math::{Scalar, ScalarBits};
use lactose_reflect::{Field, FieldKind, Persistence, Registry, TypeInfo, TypeRole};

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
        Self::over(
            registry
                .iter()
                .filter(|info| info.persistence() != Persistence::Derived)
                .map(|info| (info, LayoutDigest::of(info))),
        )
    }

    /// The hash over `saved`, ascending by key, each type with its digest: the one stream
    /// [`SchemaHash::of`] and [`SchemaTable::from_registry`] both hash.
    fn over<'a>(saved: impl Iterator<Item = (&'a TypeInfo, LayoutDigest)>) -> Self {
        let mut out = Vec::new();
        for (info, layout) in saved {
            out.extend_from_slice(&info.key().0.to_le_bytes());
            out.extend_from_slice(&info.version().to_le_bytes());
            out.push(match info.role() {
                TypeRole::Component => 0,
                TypeRole::Resource => 1,
            });
            out.push(match info.persistence() {
                Persistence::Persistent => 0,
                Persistence::Transient => 1,
                Persistence::Derived => unreachable!("a derived type is never hashed"),
            });
            out.extend_from_slice(&layout.0.to_le_bytes());
        }
        Self(xxh64(&out, 0))
    }
}

/// What the table keeps for a saved type.
#[derive(Clone, Debug)]
pub(crate) struct Saved {
    /// The schema, its layout digest included.
    pub(crate) schema: ComponentSchema,
    /// The offset of every `Scalar` leaf at any depth, ascending: what
    /// [`Saved::canonicalise`] rewrites.
    scalars: Vec<usize>,
}

impl Saved {
    /// The layout digest the schema carries.
    pub(crate) fn layout(&self) -> LayoutDigest {
        LayoutDigest(self.schema.layout_digest())
    }

    /// Rewrites every `Scalar` leaf of `value`, one value of this type, to its canonical
    /// bits (`README.md`, "Determinism"). `value` is the schema's size.
    pub(crate) fn canonicalise(&self, value: &mut [u8]) {
        for &offset in &self.scalars {
            let leaf = &mut value[offset..offset + Scalar::BYTES];
            let bits = ScalarBits::from_le_bytes((&*leaf).try_into().expect("a scalar's width"));
            let canonical = Scalar::from_canonical_bits(bits).to_canonical_bits();
            leaf.copy_from_slice(&canonical.to_le_bytes());
        }
    }

    /// Asserts, in a debug build, that no `Scalar` leaf of `value` is a NaN: one in a
    /// persistent value of a live world is a defect of the build that wrote it
    /// (architecture section 8.2). For the live-world entry points alone, never for a
    /// decoded value, which [`canonicalise`](Self::canonicalise) takes as it comes. A
    /// value of another size is left to the caller's size check.
    pub(crate) fn debug_assert_no_nan(&self, value: &[u8], name: &str) {
        if !cfg!(debug_assertions) {
            return;
        }
        for &offset in &self.scalars {
            let Some(leaf) = value.get(offset..offset + Scalar::BYTES) else { return };
            let bits = ScalarBits::from_le_bytes(leaf.try_into().expect("a scalar's width"));
            assert!(
                !Scalar::from_canonical_bits(bits).is_nan(),
                "a NaN at byte {offset} of a persistent `{name}` in a live world's export: \
                 simulation state never holds one (architecture section 8.2)"
            );
        }
    }
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
/// The schema of a type is `lactose_ecs::document_schema` of it, the one the ECS loader
/// compares a document's against (`World::load_region` refuses a document whose schema
/// differs with `VersionMismatch`): the component key, the schema version, the layout's
/// size, the [`LayoutDigest`], and one [`FieldLayout`](lactose_bundle::FieldLayout) per
/// non-padding top-level field with its ID, offset and size, in field-ID order.
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
    /// `lactose_bundle::MAX_COMPONENT_BYTES` (16 KiB), the one size a document carries,
    /// as `WorldBuilder::build` refuses a saved resource of that size, so neither reaches
    /// a world.
    pub fn from_registry(registry: &Registry) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for info in registry.iter() {
            let saved = match info.persistence() {
                Persistence::Derived => None,
                Persistence::Persistent | Persistence::Transient => Some(saved_of(info)?),
            };
            entries.insert(info.key(), Entry { info, saved });
        }
        let hash = SchemaHash::over(
            entries.values().filter_map(|entry| Some((entry.info, entry.saved.as_ref()?.layout()))),
        );
        Ok(Self { entries, hash })
    }

    /// What the table keeps for `component`: `None` for a type that is not registered or
    /// is `derived`.
    pub(crate) fn saved(&self, component: ComponentKey) -> Option<&Saved> {
        self.entries.get(&component)?.saved.as_ref()
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

    /// The layout digest of `component`, [`LayoutDigest::of`] its description and the one
    /// its schema carries: `None` for a type that is not registered or is `derived`.
    #[must_use]
    pub fn layout(&self, component: ComponentKey) -> Option<LayoutDigest> {
        Some(self.entries.get(&component)?.saved.as_ref()?.layout())
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

/// What the table keeps for the saved type `info`, after the refusals
/// [`SchemaTable::from_registry`] states.
fn saved_of(info: &TypeInfo) -> Result<Saved> {
    let size = info.layout().size();
    if size > MAX_COMPONENT_BYTES as usize {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "`{}` is saved but is {size} bytes, over the {MAX_COMPONENT_BYTES} a document \
                 carries",
                info.name()
            ),
        ));
    }
    let mut scalars = Vec::new();
    for field in info.layout().fields() {
        let (kind, tag) = (field.kind(), field.type_name());
        scalar_leaves(info.name(), &kind, tag, field.offset(), field.size(), &mut scalars)?;
    }
    Ok(Saved { schema: document_schema(info)?, scalars })
}

/// Collects the offset of every `Scalar` leaf of a field of `kind` at `offset`, refusing a
/// raw float and a foreign scalar at any depth, as `lactose-sim`'s registration does.
fn scalar_leaves(
    name: &str,
    kind: &FieldKind,
    tag: &str,
    offset: usize,
    size: usize,
    out: &mut Vec<usize>,
) -> Result<()> {
    let refuse = |what: String| {
        Err(Error::new(
            ErrorKind::Unsupported,
            format!(
                "`{name}` is saved but holds {what}; make it `derived` or use \
                 `lactose_math::Scalar`"
            ),
        ))
    };
    match kind {
        FieldKind::Float => return refuse("a raw float".to_owned()),
        FieldKind::Scalar { .. } if tag != <Scalar as Field>::NAME || size != Scalar::BYTES => {
            return refuse(format!("the scalar `{tag}`"));
        }
        FieldKind::Scalar { .. } => out.push(offset),
        FieldKind::Struct(info) => {
            for field in info.fields() {
                let (kind, at) = (field.kind(), offset + field.offset());
                scalar_leaves(name, &kind, field.type_name(), at, field.size(), out)?;
            }
        }
        FieldKind::Array { element, element_name, len } => {
            let element_size = size.checked_div(*len).unwrap_or(0);
            for index in 0..*len {
                let at = offset + index * element_size;
                scalar_leaves(name, element, element_name, at, element_size, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}
