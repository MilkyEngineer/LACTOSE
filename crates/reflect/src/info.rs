//! Type descriptions, and access to a value's bytes by field ID.
//!
//! Descriptions are built in constant evaluation from the type itself: sizes, alignments and
//! validity checks come from the compiler through the constructors here, and offsets from
//! `offset_of!` in the macro expansion. Nothing is computed by macro arithmetic.

use core::fmt::Write;
use core::marker::PhantomData;
use core::ops::Deref;

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_core::{ComponentKey, EntityRef, Error, ErrorKind, FieldId, Result};

use crate::Reflect;
use crate::field::{Field, FieldKind, PaddingField};

/// Whether a type's values are saved and hashed (architecture section 15.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Persistence {
    /// Saved at full precision, and hashed.
    Persistent,
    /// Written before it is read every tick; never saved, but hashed. The ECS zeroes
    /// transient columns every tick, so the all-zero value must be valid.
    Transient,
    /// Rebuilt on load; neither saved nor hashed.
    Derived,
}

/// Which consumers follow a column's changes row by row (architecture section 6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Tracking {
    /// Sent to clients by replication.
    pub replicated: bool,
    /// Copied to the render side by extract.
    pub extracted: bool,
}

/// Whether a registered type is a component or a resource.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TypeRole {
    /// One value per entity.
    Component,
    /// A singleton.
    Resource,
}

fn is_valid<T: CheckedBitPattern>(bytes: &[u8]) -> bool {
    bytemuck::checked::try_pod_read_unaligned::<T>(bytes).is_ok()
}

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// One field of a struct, padding fields included.
#[derive(Clone, Copy, Debug)]
pub struct FieldInfo {
    id: Option<FieldId>,
    name: &'static str,
    type_name: &'static str,
    offset: usize,
    size: usize,
    kind: FieldKind,
    valid: fn(&[u8]) -> bool,
}

impl FieldInfo {
    /// The field `name` of type `T`, declared `#[field(id = N)]`, at byte `offset`.
    #[must_use]
    pub const fn new<T: Field>(id: FieldId, name: &'static str, offset: usize) -> Self {
        Self {
            id: Some(id),
            name,
            type_name: T::NAME,
            offset,
            size: size_of::<T>(),
            kind: T::KIND,
            valid: is_valid::<T>,
        }
    }

    /// The `#[field(padding)]` field `name` of type `P` at byte `offset`.
    #[must_use]
    pub const fn padding<P: PaddingField>(name: &'static str, offset: usize) -> Self {
        Self {
            id: None,
            name,
            type_name: "padding",
            offset,
            size: size_of::<P>(),
            kind: FieldKind::Padding,
            valid: is_valid::<P>,
        }
    }

    /// The declared ID, or `None` for padding.
    #[must_use]
    pub const fn id(&self) -> Option<FieldId> {
        self.id
    }

    /// The field's Rust name, or its index for a tuple field.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The stable tag of the field's type, its [`Field::NAME`].
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Byte offset within the struct.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Size in bytes.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
    }

    /// What the field holds.
    #[must_use]
    pub const fn kind(&self) -> FieldKind {
        self.kind
    }

    /// Whether this is a padding field.
    #[must_use]
    pub const fn is_padding(&self) -> bool {
        self.id.is_none()
    }

    fn range(&self) -> core::ops::Range<usize> {
        self.offset..self.offset.saturating_add(self.size)
    }
}

/// The layout of a struct: a component, a resource or a nested `#[value]` struct.
#[derive(Clone, Copy, Debug)]
pub struct StructInfo {
    name: &'static str,
    size: usize,
    align: usize,
    fields: &'static [FieldInfo],
    retired: &'static [FieldId],
    valid: fn(&[u8]) -> bool,
}

impl StructInfo {
    /// The layout of the nested struct `T`, whose Rust name is `name`, whose fields in
    /// declaration order are `fields`, and whose deleted fields had the IDs in `retired`.
    ///
    /// A `StructInfo` describes a [`FieldKind::Struct`] for the schema dump and for access by
    /// field ID. Validation of a registered type never consults one.
    #[must_use]
    pub const fn new<T: NoUninit + CheckedBitPattern>(
        name: &'static str,
        fields: &'static [FieldInfo],
        retired: &'static [FieldId],
    ) -> Self {
        Self {
            name,
            size: size_of::<T>(),
            align: align_of::<T>(),
            fields,
            retired,
            valid: is_valid::<T>,
        }
    }

    /// The struct's Rust name, or the declared stable name for a component or resource.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Size in bytes.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
    }

    /// Alignment in bytes.
    #[must_use]
    pub const fn align(&self) -> usize {
        self.align
    }

    /// Every field in declaration order, which is also offset order. Padding is included.
    #[must_use]
    pub const fn fields(&self) -> &'static [FieldInfo] {
        self.fields
    }

    /// The IDs of fields this struct once had. They are never given to a new field.
    #[must_use]
    pub const fn retired(&self) -> &'static [FieldId] {
        self.retired
    }

    /// The field declared with `id`.
    pub fn field(&self, id: FieldId) -> Result<&'static FieldInfo> {
        self.fields.iter().find(|field| field.id == Some(id)).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("`{}` has no field ID {}", self.name, id.0))
        })
    }

    /// Checks that the description is self-consistent: fields tile the struct exactly, every
    /// non-padding field has an ID, no ID repeats or is retired, every field has a size its
    /// kind allows, and nested layouts pass the same checks.
    pub(crate) fn check(&self) -> Result<()> {
        let name = self.name;
        let mut end = 0;
        for (index, field) in self.fields.iter().enumerate() {
            if field.offset != end {
                return Err(invalid(format!(
                    "`{name}`: field `{}` is at offset {} but the previous field ends at {end}",
                    field.name, field.offset
                )));
            }
            end = field.range().end;
            match (field.id, field.kind) {
                (None, FieldKind::Padding) => {}
                (None, _) => {
                    return Err(invalid(format!(
                        "`{name}`: field `{}` has no field ID",
                        field.name
                    )));
                }
                (Some(_), FieldKind::Padding) => {
                    return Err(invalid(format!(
                        "`{name}`: padding field `{}` has a field ID",
                        field.name
                    )));
                }
                (Some(id), _) => {
                    if let Some(first) = self.fields[..index].iter().find(|f| f.id == Some(id)) {
                        return Err(invalid(format!(
                            "`{name}`: field ID {} is used by both `{}` and `{}`",
                            id.0, first.name, field.name
                        )));
                    }
                    if self.retired.contains(&id) {
                        return Err(invalid(format!(
                            "`{name}`: field `{}` uses the retired field ID {}",
                            field.name, id.0
                        )));
                    }
                }
            }
            if !kind_fits(&field.kind, field.size) {
                return Err(invalid(format!(
                    "`{name}`: field `{}` of type `{}` is {} bytes, which its kind does not allow",
                    field.name, field.type_name, field.size
                )));
            }
            check_nested(&field.kind)?;
        }
        if end != self.size {
            return Err(invalid(format!(
                "`{name}`: fields cover {end} bytes of a {}-byte struct",
                self.size
            )));
        }
        if self.align == 0 || !self.size.is_multiple_of(self.align) {
            return Err(invalid(format!(
                "`{name}`: size {} is not a multiple of alignment {}",
                self.size, self.align
            )));
        }
        for (index, id) in self.retired.iter().enumerate() {
            if self.retired[..index].contains(id) {
                return Err(invalid(format!("`{name}`: field ID {} is retired twice", id.0)));
            }
        }
        Ok(())
    }

    fn check_len(&self, value: &[u8]) -> Result<()> {
        if value.len() == self.size {
            Ok(())
        } else {
            Err(invalid(format!("`{}` is {} bytes, got {}", self.name, self.size, value.len())))
        }
    }
}

/// Whether a field of `size` bytes can hold `kind`.
fn kind_fits(kind: &FieldKind, size: usize) -> bool {
    match kind {
        FieldKind::Bool => size == 1,
        FieldKind::Unsigned | FieldKind::Signed => matches!(size, 1 | 2 | 4 | 8),
        FieldKind::Float | FieldKind::Scalar => matches!(size, 4 | 8),
        FieldKind::EntityRef => size == size_of::<EntityRef>(),
        // The raw identity of `AssetReference` is 64 bits.
        FieldKind::AssetRef => size == 8,
        FieldKind::Enum(info) => size == info.size && matches!(size, 1 | 2 | 4),
        FieldKind::Struct(info) => size == info.size,
        FieldKind::Array { element, len, .. } => match size.checked_div(*len) {
            Some(element_size) => size == element_size * len && kind_fits(element, element_size),
            None => size == 0,
        },
        FieldKind::Padding => true,
    }
}

fn check_nested(kind: &FieldKind) -> Result<()> {
    match kind {
        FieldKind::Struct(info) => info.check(),
        FieldKind::Array { element, .. } => check_nested(element),
        _ => Ok(()),
    }
}

fn zero_padding(fields: &[FieldInfo], value: &mut [u8]) {
    for field in fields {
        if let Some(bytes) = value.get_mut(field.range()) {
            zero_padding_of_kind(&field.kind, bytes);
        }
    }
}

fn zero_padding_of_kind(kind: &FieldKind, bytes: &mut [u8]) {
    match kind {
        FieldKind::Padding => bytes.fill(0),
        FieldKind::Struct(info) => zero_padding(info.fields, bytes),
        FieldKind::Array { element, len, .. } => {
            let element_size = bytes.len().checked_div(*len).unwrap_or(0);
            if element_size > 0 {
                for bytes in bytes.chunks_exact_mut(element_size) {
                    zero_padding_of_kind(element, bytes);
                }
            }
        }
        _ => {}
    }
}

/// The description of a component or resource type.
///
/// One exists only inside a [`Schema<T>`]. See [`TypeInfo::validate`] for which descriptions
/// may be relied on, and for what.
#[derive(Clone, Copy, Debug)]
pub struct TypeInfo {
    name: &'static str,
    key: ComponentKey,
    role: TypeRole,
    persistence: Persistence,
    version: u32,
    tracking: Tracking,
    layout: StructInfo,
}

impl TypeInfo {
    const fn new(
        name: &'static str,
        role: TypeRole,
        persistence: Persistence,
        version: u32,
        tracking: Tracking,
        layout: StructInfo,
    ) -> Self {
        Self {
            name,
            key: ComponentKey::from_name(name),
            role,
            persistence,
            version,
            tracking,
            layout,
        }
    }

    /// The declared stable name, such as `"game::health"`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The build, wire and disk key.
    #[must_use]
    pub const fn key(&self) -> ComponentKey {
        self.key
    }

    /// Component or resource.
    #[must_use]
    pub const fn role(&self) -> TypeRole {
        self.role
    }

    /// The persistence policy.
    #[must_use]
    pub const fn persistence(&self) -> Persistence {
        self.persistence
    }

    /// The schema version, which a migration targets.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Which consumers track this type's changes per row.
    #[must_use]
    pub const fn tracking(&self) -> Tracking {
        self.tracking
    }

    /// Size, alignment and fields.
    #[must_use]
    pub const fn layout(&self) -> &StructInfo {
        &self.layout
    }

    /// The field declared with `id`.
    pub fn field(&self, id: FieldId) -> Result<&'static FieldInfo> {
        self.layout.field(id)
    }

    /// The bytes of field `id` within `value`, the bytes of one value of this type.
    pub fn field_bytes<'a>(&self, value: &'a [u8], id: FieldId) -> Result<&'a [u8]> {
        self.layout.check_len(value)?;
        let field = self.layout.field(id)?;
        value.get(field.range()).ok_or_else(|| self.out_of_bounds(field))
    }

    /// Overwrites field `id` within `value` with `bytes`, after checking that `bytes` is a
    /// valid value of the field type the description names. `value` is unchanged on error.
    ///
    /// This is a decoding step, not a proof. The check itself is the field type's own
    /// `CheckedBitPattern`, but which type sits at which offset is a statement of the
    /// `Reflect` impl, which only the attribute macros are known to get right. A decoder
    /// finishes with [`TypeInfo::validate`], and only that result may be relied on.
    pub fn write_field(&self, value: &mut [u8], id: FieldId, bytes: &[u8]) -> Result<()> {
        self.layout.check_len(value)?;
        let field = self.layout.field(id)?;
        if bytes.len() != field.size {
            return Err(invalid(format!(
                "field `{}` of `{}` is {} bytes, got {}",
                field.name,
                self.name,
                field.size,
                bytes.len()
            )));
        }
        if !(field.valid)(bytes) {
            return Err(invalid(format!(
                "invalid bit pattern for field `{}` of `{}`",
                field.name, self.name
            )));
        }
        value
            .get_mut(field.range())
            .ok_or_else(|| self.out_of_bounds(field))?
            .copy_from_slice(bytes);
        Ok(())
    }

    /// Checks that `value` is the bytes of one valid value of the described type. Non-zero
    /// padding is invalid.
    ///
    /// # What code with `unsafe` may rely on
    ///
    /// Take a description from `T::SCHEMA`, or from [`Registry::get`](crate::Registry::get)
    /// or [`Registry::get_by_type`](crate::Registry::get_by_type) for a type registered as
    /// `T`. Then `layout().size()` and `layout().align()` are `T`'s, and `validate(b).is_ok()`
    /// means that `b.len() == size_of::<T>()` and that `T`'s own `CheckedBitPattern` accepts
    /// `b`. This holds because [`Schema::new`] takes all three from `T`, whatever else it is
    /// given, and it rests on `T`'s `CheckedBitPattern` impl alone.
    ///
    /// Nothing else is a proof: not the field list, [`TypeInfo::write_field`] or
    /// [`TypeInfo::zero_padding`], not the visitors, not any [`Field::KIND`] or
    /// [`Field::NAME`], and not a description reached any other way. Safe code can build a
    /// `Schema<U>` under any name, so a description is evidence about `T` only when it came
    /// from `T` as above; the key inside it identifies nothing by itself.
    pub fn validate(&self, value: &[u8]) -> Result<()> {
        self.layout.check_len(value)?;
        if (self.layout.valid)(value) {
            Ok(())
        } else {
            Err(invalid(format!("invalid bit pattern for `{}`", self.name)))
        }
    }

    /// Zeroes every padding byte of `value`, including padding inside nested structs.
    ///
    /// Where the padding lies is read from the description, so this proves nothing about
    /// `value`; run [`TypeInfo::validate`] afterwards.
    pub fn zero_padding(&self, value: &mut [u8]) -> Result<()> {
        self.layout.check_len(value)?;
        zero_padding(self.layout.fields, value);
        Ok(())
    }

    fn out_of_bounds(&self, field: &FieldInfo) -> Error {
        invalid(format!("field `{}` of `{}` lies outside the value", field.name, self.name))
    }

    /// Appends this type's schema to `out`; see [`Registry::dump`](crate::Registry::dump).
    pub(crate) fn dump(&self, out: &mut String) {
        let role = match self.role {
            TypeRole::Component => "component",
            TypeRole::Resource => "resource",
        };
        let persistence = match self.persistence {
            Persistence::Persistent => "persistent",
            Persistence::Transient => "transient",
            Persistence::Derived => "derived",
        };
        let _ = write!(
            out,
            "{role} {} key={:#018x} version={} {persistence} size={}",
            self.name, self.key.0, self.version, self.layout.size
        );
        if self.tracking.replicated {
            out.push_str(" replicated");
        }
        if self.tracking.extracted {
            out.push_str(" extracted");
        }
        out.push('\n');
        for field in self.layout.fields {
            out.push_str("  ");
            dump_field(field, out);
            out.push('\n');
        }
        if !self.layout.retired.is_empty() {
            out.push_str("  ");
            dump_retired(self.layout.retired, out);
            out.push('\n');
        }
    }
}

fn dump_field(field: &FieldInfo, out: &mut String) {
    match field.id {
        Some(id) => {
            let _ = write!(out, "{} ", id.0);
        }
        None => out.push_str("- "),
    }
    let _ = write!(out, "{}: ", field.name);
    dump_type(&field.kind, field.type_name, out);
    let _ = write!(out, " size={}", field.size);
}

fn dump_retired(retired: &[FieldId], out: &mut String) {
    out.push_str("retired");
    let mut ids: Vec<u16> = retired.iter().map(|id| id.0).collect();
    ids.sort_unstable();
    for id in ids {
        let _ = write!(out, " {id}");
    }
}

/// Composite types are written out structurally; every other type is its tag.
fn dump_type(kind: &FieldKind, type_name: &str, out: &mut String) {
    match kind {
        FieldKind::Enum(info) => {
            let _ = write!(out, "enum {} {{", info.name);
            for (index, variant) in info.variants.iter().enumerate() {
                let separator = if index == 0 { " " } else { ", " };
                let _ = write!(out, "{separator}{} = {}", variant.name, variant.value);
            }
            out.push_str(" }");
        }
        FieldKind::Struct(info) => {
            let _ = write!(out, "struct {} {{", info.name);
            let mut separator = " ";
            for field in info.fields {
                out.push_str(separator);
                dump_field(field, out);
                separator = ", ";
            }
            if !info.retired.is_empty() {
                out.push_str(separator);
                dump_retired(info.retired, out);
            }
            out.push_str(" }");
        }
        FieldKind::Array { element, element_name, len } => {
            out.push('[');
            dump_type(element, element_name, out);
            let _ = write!(out, "; {len}]");
        }
        _ => out.push_str(type_name),
    }
}

/// The [`TypeInfo`] of `T`, tied to `T` by construction.
///
/// [`Reflect::SCHEMA`] has this type, and [`Schema::new`] is the only way to make one or
/// the [`TypeInfo`] inside it, so the size, alignment and whole-value validity check of the
/// description are always `T`'s own, whatever the rest of it says. It dereferences to the
/// [`TypeInfo`].
#[derive(Debug)]
pub struct Schema<T> {
    info: TypeInfo,
    owner: PhantomData<fn() -> T>,
}

impl<T: NoUninit + CheckedBitPattern> Schema<T> {
    /// The schema of `T`, declared under the stable `name`, whose fields in declaration
    /// order are `fields` and whose deleted fields had the IDs in `retired`. The key is the
    /// hash of `name`; size, alignment and the validity check are `T`'s.
    #[must_use]
    pub const fn new(
        name: &'static str,
        role: TypeRole,
        persistence: Persistence,
        version: u32,
        tracking: Tracking,
        fields: &'static [FieldInfo],
        retired: &'static [FieldId],
    ) -> Self {
        let layout = StructInfo::new::<T>(name, fields, retired);
        Self {
            info: TypeInfo::new(name, role, persistence, version, tracking, layout),
            owner: PhantomData,
        }
    }
}

impl<T> Schema<T> {
    /// The description, as the registry stores it.
    #[must_use]
    pub const fn info(&'static self) -> &'static TypeInfo {
        &self.info
    }
}

impl<T> Deref for Schema<T> {
    type Target = TypeInfo;

    fn deref(&self) -> &TypeInfo {
        &self.info
    }
}

/// Reads one value of `T` from `bytes`, which need not be aligned. This is the validation
/// that save load and network receive run; an invalid bit pattern is an error, never a panic.
///
/// It is `bytemuck`'s checked read and involves no description, so its result rests on
/// `T`'s `CheckedBitPattern` impl alone.
///
/// It is `bytemuck`'s checked read and involves no description, so its result rests on
/// `T`'s `CheckedBitPattern` impl alone.
pub fn decode<T: Reflect>(bytes: &[u8]) -> Result<T> {
    bytemuck::checked::try_pod_read_unaligned(bytes).map_err(|_| {
        invalid(format!("{} bytes are not a valid `{}`", bytes.len(), T::SCHEMA.name()))
    })
}
