//! The layout of the components a document carries, so the document describes itself.

use std::collections::BTreeSet;

use lactose_core::{ComponentKey, FieldId, Result};

use crate::MAX_COMPONENT_BYTES;
use crate::codec::{Reader, Writer, invalid, strictly_ascending};

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
/// its size and where each field lies.
///
/// Every document carries the schema of every component it holds data for. That is what
/// lets a save written under an old schema be decoded field by field (architecture section
/// 15.4) without the reading build knowing historical layouts, and what lets a `SetField`
/// record name a field the document itself can locate.
///
/// The caller supplies schemas as plain data; they come from the reflection registry. This
/// crate checks that a schema is well-formed and that the bytes stored under it have its
/// size. It never looks inside those bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ComponentSchema {
    component: ComponentKey,
    schema_version: u32,
    element_size: u32,
    fields: Vec<FieldLayout>,
}

impl ComponentSchema {
    /// A schema for `component` at `schema_version`, whose values are `element_size` bytes
    /// with `fields` given in any order. Bytes no field covers are padding.
    ///
    /// Fails with `ErrorKind::InvalidData` if `element_size` is larger than
    /// [`MAX_COMPONENT_BYTES`], if two fields share an ID or overlap, or if a field is
    /// empty or reaches past the end of the component.
    pub fn new(
        component: ComponentKey,
        schema_version: u32,
        element_size: u32,
        fields: impl IntoIterator<Item = FieldLayout>,
    ) -> Result<Self> {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.sort_by_key(|field| field.id);
        let schema = Self { component, schema_version, element_size, fields };
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

pub(crate) fn write_table(writer: &mut Writer, schemas: &[ComponentSchema]) {
    writer.count(schemas.len());
    for schema in schemas {
        writer.u64(schema.component.0);
        writer.u32(schema.schema_version);
        writer.u32(schema.element_size);
        writer.count(schema.fields.len());
        for field in &schema.fields {
            writer.u16(field.id.0);
            writer.u32(field.offset);
            writer.u32(field.size);
        }
    }
}

/// Reads a schema table and checks each schema. Order and use are checked by
/// [`check_table`] once the document is whole.
pub(crate) fn read_table(reader: &mut Reader) -> Result<Vec<ComponentSchema>> {
    // A schema is at least its key, version, size and field count; a field is ten bytes.
    let count = reader.count(20)?;
    let mut schemas = Vec::with_capacity(count);
    for _ in 0..count {
        let (component, schema_version, element_size) =
            (reader.component()?, reader.u32()?, reader.u32()?);
        let field_count = reader.count(10)?;
        let mut fields = Vec::with_capacity(field_count);
        for _ in 0..field_count {
            fields.push(FieldLayout {
                id: FieldId(reader.u16()?),
                offset: reader.u32()?,
                size: reader.u32()?,
            });
        }
        let schema = ComponentSchema { component, schema_version, element_size, fields };
        schema.validate()?;
        schemas.push(schema);
    }
    Ok(schemas)
}
