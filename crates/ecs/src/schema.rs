//! The schema a document carries for a registered type (architecture section 15.4), and the
//! layout digest inside it: the one derivation of both, for this crate's loader and for
//! `lactose-persist`'s schema table.

use lactose_bundle::{ComponentSchema, FieldLayout};
use lactose_core::{Result, xxh64};
use lactose_reflect::{FieldKind, StructInfo, TypeInfo};

/// The digest of a type's layout at one schema version: what the rest of a
/// [`ComponentSchema`] does not see, in one number, carried in it as
/// [`ComponentSchema::layout_digest`].
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
/// The rest of a schema holds the key, the version, the size and each top-level field's
/// ID, offset and size, so three kinds of change leave it equal: a reorder inside a nested
/// `#[value]` struct, renumbered enum discriminants, and a same-size type change (`i32` to
/// `u32`; `u64` to `Scalar` on a 64-bit scalar build, `u32` on a 32-bit one). The digest
/// sees all three, so [`World::load_region`](crate::World::load_region) refuses a baseline
/// or an overlay written under another layout at the same version, and
/// `lactose-persist`'s migration refuses equal versions with different digests. Free,
/// because no saved byte depends on them: renaming a field (its ID stays) or a `#[value]`
/// struct or enum type. Not free: renaming an enum variant, which changes the digest and so
/// is a version bump with a step that changes nothing (the saved value means the same
/// variant under its new name), and so is reordering the variants' declarations, since the
/// pairs are written in declaration order; the bump is the price of seeing a permutation.
/// What no digest sees is a change of meaning under an unchanged layout (a field's unit;
/// one `#[value]` struct replaced by another of the same layout; one enum by another of the
/// same size with the same (name, value) pairs in the same order; one primitive by another
/// of the same tag): only a version bump and a step carry that, and the build-time guard
/// D129 names (a committed record of each saved type's version and digest, compared by a
/// test that fails when the digest changed and the version did not) is what makes the bump
/// hard to forget.
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

/// The schema a document must carry for `info`, the one derivation of it: the component
/// key, the schema version, the layout's size, the [`LayoutDigest`], and one
/// [`FieldLayout`] per non-padding top-level field with its ID, offset and size, in field-ID
/// order. Padding fields and nested structure are not in the field list; a nested
/// `#[value]` struct is one field of its outer size, and the digest sees inside it.
///
/// [`World::load_region`](crate::World::load_region) refuses a document whose schema of a
/// type is not this one, and `lactose-persist`'s schema table holds this one for every
/// saved type. Fails as [`ComponentSchema::new`] fails, which for a registered type is
/// only a layout over `lactose_bundle::MAX_COMPONENT_BYTES`: a component is capped there
/// when a world is built and a saved resource is refused there, but a `derived` resource,
/// which no document carries, may be larger.
pub fn document_schema(info: &TypeInfo) -> Result<ComponentSchema> {
    let fields = info.layout().fields().iter().filter_map(|field| {
        Some(FieldLayout {
            id: field.id()?,
            offset: field.offset() as u32,
            size: field.size() as u32,
        })
    });
    let size = info.layout().size() as u32;
    ComponentSchema::new(info.key(), info.version(), size, LayoutDigest::of(info).0, fields)
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
