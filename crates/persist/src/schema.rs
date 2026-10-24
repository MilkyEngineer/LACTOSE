//! The persistence policy as this crate reads it from the registry (architecture sections
//! 15.1 and 15.4): which types a save carries, the [`ComponentSchema`] each document must
//! carry for them, the layout digest that tells apart what the schema cannot, and the
//! schema hash a container records.

use std::collections::BTreeMap;

use lactose_bundle::{ComponentSchema, FieldLayout, MAX_COMPONENT_BYTES};
use lactose_core::{ComponentKey, Error, ErrorKind, Result, xxh64};
use lactose_math::{Scalar, ScalarBits};
use lactose_reflect::{Field, FieldKind, Persistence, Registry, StructInfo, TypeInfo, TypeRole};

/// The digest of a type's layout at one schema version: what a document's
/// [`ComponentSchema`] does not see, in one number.
///
/// It is the xxh64, seed zero, over the layout part of
/// [`TypeInfo::write_schema`](lactose_reflect::TypeInfo::write_schema), the bytes that
/// encoding appends after its first fifteen (the key, the version, the role, the
/// persistence and the tracking bits), that is, from the layout's size on: size,
/// alignment, the count of non-padding fields, and each field's ID, offset, size, type tag
/// and kind, with nested layouts, enum sizes and discriminants, and array lengths and
/// element kinds written out; with two changes. A field, or an array element, of kind
/// `Struct` or `Enum` contributes an empty type tag (a zero length and no bytes) in place
/// of its Rust name, since its structure follows and a name is not a layout; a primitive's
/// tag stays, since it is what tells `u32` from `RegionId`. And an enum's variants are
/// written, in declaration order, as each variant's value (`u32`) followed by its name (a
/// `u32` length and the UTF-8 bytes), where `write_schema` writes the values alone: a
/// saved discriminant means the variant of that name, so the name is part of the layout,
/// and a permutation of discriminants among the variants, or of their names, is seen. The
/// tracking bits are left out on purpose: marking a type replicated or extracted changes
/// no saved byte.
///
/// A `ComponentSchema` holds the key, the version, the size and each top-level field's ID,
/// offset and size, so three kinds of change leave it equal: a reorder inside a nested
/// `#[value]` struct, renumbered enum discriminants, and a same-size type change (`i32` to
/// `u32`; `u64` to `Scalar` on a 64-bit scalar build, `u32` on a 32-bit one). The digest
/// sees all three, so a save carries it beside each saved type's schema and
/// [`Migrations::migrate`](crate::Migrations::migrate) refuses equal versions with
/// different digests. Free, because no saved byte depends on them: renaming a field (its
/// ID stays) or a `#[value]` struct or enum type. Not free: renaming an enum variant,
/// which changes the digest and so is a version bump with a step that changes nothing
/// (the saved value means the same variant under its new name), and so is reordering the
/// variants' declarations, since the pairs are written in declaration order; the bump is
/// the price of seeing a permutation. What no digest sees is a change of meaning under an
/// unchanged layout (a field's unit; one `#[value]` struct replaced by another of the same
/// layout; one enum by another of the same size with the same (name, value) pairs in the
/// same order; one primitive by another of the same tag): only a version bump and a step
/// carry that, and the build-time guard D129 names (a committed record of each saved
/// type's version and digest, compared by a test that fails when the digest changed and
/// the version did not) is what makes the bump hard to forget.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LayoutDigest(pub u64);

impl LayoutDigest {
    /// The digest of `info`'s layout, as stated on the type: the implementation walks the
    /// layout as `write_schema` does and writes the same bytes, with empty tags for
    /// `Struct` and `Enum` kinds and each variant's name beside its value.
    #[must_use]
    pub fn of(info: &TypeInfo) -> Self {
        let mut out = Vec::new();
        write_layout(info.layout(), &mut out);
        Self(xxh64(&out, 0))
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
    pub(crate) schema: ComponentSchema,
    pub(crate) layout: LayoutDigest,
    /// The offset of every `Scalar` leaf at any depth, ascending: what
    /// [`Saved::canonicalise`] rewrites.
    scalars: Vec<usize>,
}

impl Saved {
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
        let mut entries = BTreeMap::new();
        for info in registry.iter() {
            let saved = match info.persistence() {
                Persistence::Derived => None,
                Persistence::Persistent | Persistence::Transient => Some(saved_of(info)?),
            };
            entries.insert(info.key(), Entry { info, saved });
        }
        let hash = SchemaHash::over(
            entries.values().filter_map(|entry| Some((entry.info, entry.saved.as_ref()?.layout))),
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

/// The schema a document carries for `info`, derived exactly as `lactose-ecs` derives the
/// one its loader compares against (`README.md`, "Schemas"; `tests/roundtrip.rs` holds the
/// two equal). Fails as `ComponentSchema::new` fails, for a type over
/// `MAX_COMPONENT_BYTES`.
pub(crate) fn document_schema(info: &TypeInfo) -> Result<ComponentSchema> {
    let fields = info.layout().fields().iter().filter_map(|field| {
        Some(FieldLayout {
            id: field.id()?,
            offset: field.offset() as u32,
            size: field.size() as u32,
        })
    });
    ComponentSchema::new(info.key(), info.version(), info.layout().size() as u32, fields)
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
    Ok(Saved { schema: document_schema(info)?, layout: LayoutDigest::of(info), scalars })
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

/// The layout part of `TypeInfo::write_schema`, with the two changes [`LayoutDigest`]
/// states.
fn write_layout(layout: &StructInfo, out: &mut Vec<u8>) {
    out.extend_from_slice(&(layout.size() as u32).to_le_bytes());
    out.extend_from_slice(&(layout.align() as u32).to_le_bytes());
    let fields = layout.fields().iter().filter(|field| !field.is_padding());
    out.extend_from_slice(&(fields.clone().count() as u32).to_le_bytes());
    for field in fields {
        let id = field.id().expect("a non-padding field has an ID");
        out.extend_from_slice(&id.0.to_le_bytes());
        out.extend_from_slice(&(field.offset() as u32).to_le_bytes());
        out.extend_from_slice(&(field.size() as u32).to_le_bytes());
        write_tag(&field.kind(), field.type_name(), out);
        write_kind(&field.kind(), out);
    }
}

/// A primitive's type tag, or an empty one for a `Struct` or `Enum`, whose structure
/// follows.
fn write_tag(kind: &FieldKind, tag: &str, out: &mut Vec<u8>) {
    let tag = match kind {
        FieldKind::Struct(_) | FieldKind::Enum(_) => "",
        _ => tag,
    };
    out.extend_from_slice(&(tag.len() as u32).to_le_bytes());
    out.extend_from_slice(tag.as_bytes());
}

fn write_kind(kind: &FieldKind, out: &mut Vec<u8>) {
    match kind {
        FieldKind::Bool => out.push(0),
        FieldKind::Unsigned => out.push(1),
        FieldKind::Signed => out.push(2),
        FieldKind::Float => out.push(3),
        FieldKind::Scalar { .. } => out.push(4),
        FieldKind::EntityRef => out.push(5),
        FieldKind::AssetRef { .. } => out.push(6),
        FieldKind::Enum(info) => {
            out.push(7);
            out.push(info.size as u8);
            out.extend_from_slice(&(info.variants.len() as u32).to_le_bytes());
            for variant in info.variants {
                out.extend_from_slice(&variant.value.to_le_bytes());
                out.extend_from_slice(&(variant.name.len() as u32).to_le_bytes());
                out.extend_from_slice(variant.name.as_bytes());
            }
        }
        FieldKind::Struct(info) => {
            out.push(8);
            write_layout(info, out);
        }
        FieldKind::Array { element, element_name, len } => {
            out.push(9);
            out.extend_from_slice(&(*len as u32).to_le_bytes());
            write_tag(element, element_name, out);
            write_kind(element, out);
        }
        FieldKind::Padding => out.push(10),
    }
}
