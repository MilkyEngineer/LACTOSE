//! The byte rules of the container and its sections: every integer little-endian, counts
//! `u32`, sizes and offsets `u64`, and every read bounds-checked so nothing is allocated
//! or looped over on the word of a count alone. The schema table is `lactose-bundle`'s
//! encoding of one, byte for byte (`tests/format.rs` holds the two equal).

use lactose_bundle::{ComponentSchema, FieldLayout};
use lactose_core::{ComponentKey, Error, ErrorKind, FieldId, Result};

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

/// The bytes being written.
#[derive(Default)]
pub(crate) struct Writer {
    pub(crate) bytes: Vec<u8>,
}

impl Writer {
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Writes a count. Every count this crate writes is bounded far below `u32::MAX` by
    /// the section cap, which the writer checks on the whole.
    pub(crate) fn count(&mut self, count: usize) {
        self.u32(u32::try_from(count).unwrap_or(u32::MAX));
    }

    /// `lactose-bundle`'s schema table: the count, then per schema its key, version, size,
    /// field count and fields (ID, offset, size), fields ascending by ID.
    pub(crate) fn schemas(&mut self, schemas: &[ComponentSchema]) {
        self.count(schemas.len());
        for schema in schemas {
            self.u64(schema.component().0);
            self.u32(schema.schema_version());
            self.u32(schema.element_size());
            self.count(schema.fields().len());
            for field in schema.fields() {
                self.u16(field.id.0);
                self.u32(field.offset);
                self.u32(field.size);
            }
        }
    }
}

/// A cursor over untrusted bytes.
pub(crate) struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let Some((taken, rest)) = self.rest.split_at_checked(len) else {
            return Err(invalid("the bytes end early"));
        };
        self.rest = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take returned N bytes"))
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(crate) fn component(&mut self) -> Result<ComponentKey> {
        Ok(ComponentKey(self.u64()?))
    }

    /// A size or length stored as a `u64`, which must fit in what remains.
    pub(crate) fn len(&mut self) -> Result<usize> {
        match usize::try_from(self.u64()?) {
            Ok(len) if len <= self.rest.len() => Ok(len),
            _ => Err(invalid("a length exceeds the bytes that follow it")),
        }
    }

    /// A presence byte: zero or one.
    pub(crate) fn flag(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("a presence byte is neither zero nor one")),
        }
    }

    /// The number of items that follow, each at least `min_item_len` bytes (not zero).
    pub(crate) fn count(&mut self, min_item_len: usize) -> Result<usize> {
        let count = self.u32()? as usize;
        match count.checked_mul(min_item_len) {
            Some(len) if len <= self.rest.len() => Ok(count),
            _ => Err(invalid("a count exceeds the bytes that follow it")),
        }
    }

    /// A schema table as [`Writer::schemas`] writes it, each schema well-formed and its
    /// fields strictly ascending by ID, as the canonical encoding has them.
    pub(crate) fn schemas(&mut self) -> Result<Vec<ComponentSchema>> {
        // A schema is at least its key, version, size and field count; a field ten bytes.
        let count = self.count(20)?;
        let mut schemas = Vec::with_capacity(count);
        for _ in 0..count {
            let (component, version, size) = (self.component()?, self.u32()?, self.u32()?);
            let field_count = self.count(10)?;
            let mut fields = Vec::with_capacity(field_count);
            for _ in 0..field_count {
                let id = FieldId(self.u16()?);
                if fields.last().is_some_and(|last: &FieldLayout| last.id >= id) {
                    return Err(invalid("a schema's fields are not strictly ascending by ID"));
                }
                fields.push(FieldLayout { id, offset: self.u32()?, size: self.u32()? });
            }
            schemas.push(ComponentSchema::new(component, version, size, fields)?);
        }
        Ok(schemas)
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.rest.is_empty() { Ok(()) } else { Err(invalid("bytes follow the content")) }
    }
}
