//! The layout of the components a document carries, so the document describes itself.

use std::collections::BTreeSet;

use lactose_core::{ComponentKey, FieldId, Result};

use crate::MAX_COMPONENT_BYTES;
use crate::codec::{Reader, invalid, strictly_ascending};

/// Where one field lies inside its component's bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FieldLayout {
    /// The field's explicit, never-reused ID.
    pub id: FieldId,
    /// The offset of the field's first byte within the component.
    pub offset: u32,
    /// The field's size in bytes, at least one.
    pub size: u32,
}

/// The layout of one component as it was when a document was written: its schema version,
/// its size, the digest of its whole layout and where each field lies.
///
/// Every document carries the schema of every component it holds data for. That is what
/// lets a save written under an old schema be decoded field by field (architecture section
/// 15.4) without the reading build knowing historical layouts, and what lets a `SetField`
/// record name a field the document itself can locate.
///
/// The fields alone cannot tell a same-size change from the layout the bytes were written
/// in: a reorder inside a nested struct, renumbered enum discriminants and a same-size type
/// change leave every top-level field's ID, offset and size equal. The layout digest does,
/// so two schemas that differ only in it are different schemas, and a loader that compares
/// a document's schema with its own refuses such a change at one schema version.
///
/// The caller supplies schemas as plain data; they come from the reflection registry, and
/// `lactose_ecs::document_schema` is the one derivation of a registered type's schema,
/// digest included. This crate checks that a schema is well-formed and that the bytes
/// stored under it have its size. It never computes or interprets the digest, and never
/// looks inside those bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ComponentSchema {
    component: ComponentKey,
    schema_version: u32,
    element_size: u32,
    layout_digest: u64,
    fields: Vec<FieldLayout>,
}

impl ComponentSchema {
    /// A schema for `component` at `schema_version`, whose values are `element_size` bytes
    /// laid out as `layout_digest` states, with `fields` given in any order. Bytes no field
    /// covers are padding.
    ///
    /// Fails with `ErrorKind::InvalidData` if `element_size` is larger than
    /// [`MAX_COMPONENT_BYTES`], if two fields share an ID or overlap, or if a field is
    /// empty or reaches past the end of the component.
    pub fn new(
        component: ComponentKey,
        schema_version: u32,
        element_size: u32,
        layout_digest: u64,
        fields: impl IntoIterator<Item = FieldLayout>,
    ) -> Result<Self> {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.sort_by_key(|field| field.id);
        let schema = Self { component, schema_version, element_size, layout_digest, fields };
        schema.validate()?;
        Ok(schema)
    }

    /// The component this schema describes.
    #[must_use]
    pub fn component(&self) -> ComponentKey {
        self.component
    }

    /// The component's schema version when the document was written.
    #[must_use]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The size of one value in bytes. Zero for a tag component.
    #[must_use]
    pub fn element_size(&self) -> u32 {
        self.element_size
    }

    /// The digest of the component's whole layout at this schema version, as the writer's
    /// registry derived it: what tells a same-size layout change from the layout these
    /// bytes were written in, which the fields cannot.
    #[must_use]
    pub fn layout_digest(&self) -> u64 {
        self.layout_digest
    }

    /// The fields, strictly ascending by [`FieldId`].
    #[must_use]
    pub fn fields(&self) -> &[FieldLayout] {
        &self.fields
    }

    /// The layout of the field `id`, if the component had it at this schema version.
    #[must_use]
    pub fn field(&self, id: FieldId) -> Option<FieldLayout> {
        let index = self.fields.binary_search_by_key(&id, |field| field.id).ok()?;
        Some(self.fields[index])
    }

    fn validate(&self) -> Result<()> {
        if self.element_size > MAX_COMPONENT_BYTES {
            return Err(invalid("a component is larger than MAX_COMPONENT_BYTES"));
        }
        strictly_ascending(
            self.fields.iter().map(|field| field.id),
            "a schema's fields are not strictly ascending by ID",
        )?;
        let mut by_offset: Vec<_> = self.fields.iter().map(|f| (f.offset, f.size)).collect();
        by_offset.sort_unstable();
        let mut end = 0u64;
        for (offset, size) in by_offset {
            if size == 0 {
                return Err(invalid("a field is empty"));
            }
            if u64::from(offset) < end {
                return Err(invalid("two fields overlap"));
            }
            end = u64::from(offset) + u64::from(size);
            if end > u64::from(self.element_size) {
                return Err(invalid("a field reaches past the end of its component"));
            }
        }
        Ok(())
    }
}

/// The schemas among `schemas` whose component is in `used`, ascending by component.
///
/// A caller may pass its whole registry. Fails if two schemas describe one component or if
/// a used component has none.
pub(crate) fn select(
    schemas: impl IntoIterator<Item = ComponentSchema>,
    used: &BTreeSet<ComponentKey>,
) -> Result<Vec<ComponentSchema>> {
    let mut schemas: Vec<_> = schemas.into_iter().collect();
    schemas.sort_by_key(ComponentSchema::component);
    strictly_ascending(
        schemas.iter().map(ComponentSchema::component),
        "two schemas describe one component",
    )?;
    schemas.retain(|schema| used.contains(&schema.component));
    if schemas.len() != used.len() {
        return Err(invalid("a component carries data but has no schema"));
    }
    Ok(schemas)
}

/// Fails unless `schemas` is ascending and describes exactly the components in `used`.
pub(crate) fn check_table(
    schemas: &[ComponentSchema],
    used: &BTreeSet<ComponentKey>,
) -> Result<()> {
    strictly_ascending(
        schemas.iter().map(ComponentSchema::component),
        "the schema table is not strictly ascending",
    )?;
    if !schemas.iter().map(ComponentSchema::component).eq(used.iter().copied()) {
        return Err(invalid("the schema table does not match the components that carry data"));
    }
    Ok(())
}

/// The schema of `component` in a table that is ascending by component.
pub(crate) fn find(
    schemas: &[ComponentSchema],
    component: ComponentKey,
) -> Option<&ComponentSchema> {
    let index = schemas.binary_search_by_key(&component, ComponentSchema::component).ok()?;
    Some(&schemas[index])
}

/// Writes `schemas` as a document's schema table: a `u32` count, then each schema as its
/// component key (`u64`), schema version (`u32`), element size (`u32`), layout digest
/// (`u64`), field count (`u32`) and fields, each as its ID (`u16`), offset and size (`u32`
/// each), ascending by ID; every integer little-endian. A schema table is strictly
/// ascending by component in every document, and [`read_table`] refuses one that is not;
/// this writes `schemas` in the order given.
///
/// The encoding a baseline and an overlay embed, and the one `lactose-persist` writes a
/// save's resource schema table in, so the table has one encoding. It changes with the
/// documents' format version.
pub fn write_table(schemas: &[ComponentSchema], out: &mut Vec<u8>) {
    // A count over `u32::MAX` saturates, and `read_table` then refuses it: no table that
    // long fits a document.
    let count = |out: &mut Vec<u8>, count: usize| {
        out.extend_from_slice(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
    };
    count(out, schemas.len());
    for schema in schemas {
        out.extend_from_slice(&schema.component.0.to_le_bytes());
        out.extend_from_slice(&schema.schema_version.to_le_bytes());
        out.extend_from_slice(&schema.element_size.to_le_bytes());
        out.extend_from_slice(&schema.layout_digest.to_le_bytes());
        count(out, schema.fields.len());
        for field in &schema.fields {
            out.extend_from_slice(&field.id.0.to_le_bytes());
            out.extend_from_slice(&field.offset.to_le_bytes());
            out.extend_from_slice(&field.size.to_le_bytes());
        }
    }
}

/// Reads a schema table as [`write_table`] writes one from the front of `bytes`, and
/// advances `bytes` past it.
///
/// Fails with `ErrorKind::InvalidData` if the bytes end early, if a count exceeds the bytes
/// that follow it, if the table is not strictly ascending by component, or if a schema is
/// not well-formed as [`ComponentSchema::new`] states or its fields are not strictly
/// ascending by ID. Nothing is allocated or looped over on the word of a count alone. On
/// failure `bytes` is unchanged.
pub fn read_table(bytes: &mut &[u8]) -> Result<Vec<ComponentSchema>> {
    let mut reader = Reader::over(bytes);
    let schemas = read(&mut reader)?;
    *bytes = reader.rest();
    Ok(schemas)
}

/// [`read_table`] over a document's reader.
pub(crate) fn read(reader: &mut Reader) -> Result<Vec<ComponentSchema>> {
    // A schema is at least its key, version, size, digest and field count; a field is ten
    // bytes.
    let count = reader.count(28)?;
    let mut schemas = Vec::with_capacity(count);
    for _ in 0..count {
        let (component, schema_version, element_size, layout_digest) =
            (reader.component()?, reader.u32()?, reader.u32()?, reader.u64()?);
        let field_count = reader.count(10)?;
        let mut fields = Vec::with_capacity(field_count);
        for _ in 0..field_count {
            fields.push(FieldLayout {
                id: FieldId(reader.u16()?),
                offset: reader.u32()?,
                size: reader.u32()?,
            });
        }
        let schema =
            ComponentSchema { component, schema_version, element_size, layout_digest, fields };
        schema.validate()?;
        schemas.push(schema);
    }
    // Lookups in a decoded document are binary searches, so the order has to hold first.
    strictly_ascending(
        schemas.iter().map(ComponentSchema::component),
        "the schema table is not strictly ascending",
    )?;
    Ok(schemas)
}
