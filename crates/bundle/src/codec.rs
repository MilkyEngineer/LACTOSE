//! The byte-level rules both documents share.
//!
//! A document is a 4-byte magic, a `u16` format version, a zero `u16`, the body, and the
//! xxh64 of every preceding byte. All integers are little-endian and every length is a
//! `u32`. Nothing depends on the host's layout, word size or byte order.

use lactose_core::{ComponentKey, Error, ErrorKind, NetId, Result, xxh64};

use crate::MAX_COMPONENT_BYTES;

/// The format version this build writes and reads, for both documents.
const FORMAT_VERSION: u16 = 1;
const HEADER_LEN: usize = 8;
const TRAILER_LEN: usize = 8;

pub(crate) fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Fails unless `items` is strictly ascending, which also means free of duplicates.
pub(crate) fn strictly_ascending<T: Ord>(
    items: impl IntoIterator<Item = T>,
    message: &'static str,
) -> Result<()> {
    let mut items = items.into_iter();
    let Some(mut previous) = items.next() else { return Ok(()) };
    for item in items {
        if item <= previous {
            return Err(invalid(message));
        }
        previous = item;
    }
    Ok(())
}

pub(crate) fn check_component_len(len: usize) -> Result<()> {
    if len > MAX_COMPONENT_BYTES as usize {
        return Err(invalid("component data is larger than MAX_COMPONENT_BYTES"));
    }
    Ok(())
}

pub(crate) struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub(crate) fn new(magic: [u8; 4]) -> Self {
        let mut writer = Self { bytes: Vec::new() };
        writer.bytes(&magic);
        writer.u16(FORMAT_VERSION);
        writer.u16(0);
        writer
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Writes a count or a length.
    ///
    /// # Panics
    ///
    /// Panics if it does not fit a `u32`. Component data is capped far below that, so only
    /// a document of more than four thousand million rows or records gets here.
    pub(crate) fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("a bundle document holds at most u32::MAX items"));
    }

    /// Seals the document with its hash and returns the bytes with that hash.
    pub(crate) fn finish(mut self) -> (Vec<u8>, u64) {
        let hash = xxh64(&self.bytes, 0);
        self.u64(hash);
        (self.bytes, hash)
    }
}

/// A cursor over the body of an untrusted document. Every read is bounds-checked, and
/// [`Reader::count`] refuses a count the remaining bytes cannot hold, so nothing is
/// allocated or looped over on the word of a length field alone.
pub(crate) struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Checks the magic, the format version and the trailing hash, and returns a reader
    /// over the body.
    pub(crate) fn open(bytes: &'a [u8], magic: [u8; 4]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + TRAILER_LEN {
            return Err(invalid("document is shorter than its header and hash"));
        }
        let (sealed, trailer) = bytes.split_at(bytes.len() - TRAILER_LEN);
        let mut header = Self { rest: &sealed[..HEADER_LEN] };
        if header.take(4)? != magic {
            return Err(invalid("not a document of this kind"));
        }
        let version = header.u16()?;
        if version != FORMAT_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!("bundle format {version} is not the supported format {FORMAT_VERSION}"),
            ));
        }
        if header.u16()? != 0 {
            return Err(invalid("reserved header bytes are not zero"));
        }
        let recorded = u64::from_le_bytes(trailer.try_into().expect("split off eight bytes"));
        if recorded != xxh64(sealed, 0) {
            return Err(invalid("document hash does not match its contents"));
        }
        Ok(Self { rest: &sealed[HEADER_LEN..] })
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let Some((taken, rest)) = self.rest.split_at_checked(len) else {
            return Err(invalid("document is truncated"));
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

    pub(crate) fn net_id(&mut self) -> Result<NetId> {
        NetId::from_bits(self.u64()?).ok_or(invalid("zero is not a network ID"))
    }

    pub(crate) fn component(&mut self) -> Result<ComponentKey> {
        Ok(ComponentKey(self.u64()?))
    }

    /// Reads the number of items that follow, each of which occupies at least
    /// `min_item_len` bytes (which must not be zero).
    pub(crate) fn count(&mut self, min_item_len: usize) -> Result<usize> {
        let count = self.u32()? as usize;
        match count.checked_mul(min_item_len) {
            Some(len) if len <= self.rest.len() => Ok(count),
            _ => Err(invalid("a count exceeds the bytes that follow it")),
        }
    }

    /// Reads a component data length and the data.
    pub(crate) fn component_bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        check_component_len(len)?;
        self.take(len)
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.rest.is_empty() { Ok(()) } else { Err(invalid("bytes follow the document")) }
    }
}
