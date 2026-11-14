//! The pack: the file the cooker writes and the store mounts.
//!
//! A pack is payloads, then an index, then a 24-byte trailer, every integer little-endian.
//! `README.md`, "The pack format", is the layout byte for byte and every rule a mount
//! checks; this module is its one writer, [`PackBuilder`], and (hidden) its one reader, the
//! index decode a mount runs. Nothing here touches a file: a pack is built as bytes and
//! written by its caller through `lactose_io::Io::replace`.

use std::borrow::Cow;
use std::collections::BTreeMap;

use lactose_asset_api::{AssetId, AssetKind, AssetVersion, CookedAsset, CookedHeader};
use lactose_core::{Error, ErrorKind, Result, xxh64};
use lactose_io::MAX_READ_BYTES;

/// The pack format version this build writes and mounts. A pack of another version is
/// refused whole with `ErrorKind::VersionMismatch`.
///
/// The last eight bytes of a pack are the same in every format version (the magic `LPAK`,
/// this number as a `u16`, a zero `u16`), so a build always tells a pack of another
/// version from a file that is not a pack.
pub const PACK_FORMAT_VERSION: u16 = 1;

/// How [`PackBuilder::add`] is asked to store one asset's cooked bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Storage {
    /// The cooked bytes as they are.
    Raw,
    /// One zstd frame of the cooked bytes at compression level `level`, 1 to 22, kept only
    /// if it is shorter than the bytes themselves; otherwise the bytes are stored raw. The
    /// index records which, so the choice is the writer's alone and no reader names a
    /// level.
    Zstd {
        /// The zstd compression level, 1 (fastest) to 22 (smallest).
        level: i32,
    },
}

/// One entry of a pack index, as `README.md` lays it out. Hidden and changeable.
pub(crate) struct Entry {
    pub(crate) id: AssetId,
    pub(crate) version: AssetVersion,
    pub(crate) header: CookedHeader,
    /// Whether the payload is one zstd frame; raw otherwise.
    pub(crate) compressed: bool,
    pub(crate) offset: u64,
    pub(crate) stored_len: u64,
    pub(crate) decoded_len: u64,
}

/// The length of the trailer, the last bytes of every pack.
pub(crate) const TRAILER_LEN: u64 = 24;

const MAGIC: [u8; 4] = *b"LPAK";

/// An entry's length without its dependencies.
const ENTRY_LEN: u64 = 57;

/// The bytes of an entry after its dependencies: the storage code and three `u64`s.
const ENTRY_TAIL_LEN: u64 = 25;

const RAW: u8 = 0;
const ZSTD: u8 = 1;

/// Builds one pack in memory: the cooker's writer, and the tests'.
///
/// A pack holds at most one version of an asset. Its bytes are not an identity: an asset's
/// [`AssetVersion`] hashes its header and its decoded bytes, never the pack, so two packs
/// that store one asset differently publish it under one version.
///
/// ```no_run
/// use lactose_asset::{PackBuilder, Storage};
/// use lactose_asset_api::{AssetId, CookedAsset};
///
/// # fn cooked() -> CookedAsset { unimplemented!() }
/// # fn main() -> lactose_core::Result<()> {
/// let mut pack = PackBuilder::new();
/// let barrel = AssetId::from_name("props/barrel.mesh");
/// let version = pack.add(barrel, &cooked(), Storage::Zstd { level: 3 })?;
/// let bytes = pack.finish()?; // written whole: `Io::replace("cooked/0007.pack", bytes)`
/// # let _ = (version, bytes);
/// # Ok(()) }
/// ```
pub struct PackBuilder {
    /// Every payload added so far, back to back in add order. Hidden and changeable.
    payloads: Vec<u8>,
    /// One entry per ID added, in ID order. Hidden and changeable.
    entries: BTreeMap<AssetId, Entry>,
}

impl PackBuilder {
    /// An empty pack. An empty pack is a valid pack: it mounts and names nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self { payloads: Vec::new(), entries: BTreeMap::new() }
    }

    /// Adds `cooked` as this pack's version of `id` and returns that version,
    /// `cooked.version()`, which is what the index records and what a store verifies the
    /// content against when it loads it.
    ///
    /// The payload is appended after those already added. Under [`Storage::Zstd`] it is
    /// compressed here, as exactly one zstd frame that carries its content size and whose
    /// window is at most 2^27 bytes: the three things `lactose_io`'s encoded read requires
    /// of a stored range, met in this one place for every writer.
    ///
    /// Adding an ID again with content of the version already recorded for it adds nothing
    /// and returns that version, whatever `storage` says: equal content under one ID is
    /// one asset, as two regions with equal baselines are.
    ///
    /// Fails, adding nothing, with `ErrorKind::InvalidData` if `id` is `AssetId::NONE`, if
    /// `id` was added already with another version (two names that hash to one ID reach
    /// the cooker as this, which is where its collision check reports them), or if `level`
    /// is outside 1 to 22; with `ErrorKind::ResourceExhausted` if `cooked.bytes` is longer
    /// than `lactose_io::MAX_READ_BYTES`, which no store could read back, or if zstd
    /// cannot allocate.
    pub fn add(
        &mut self,
        id: AssetId,
        cooked: &CookedAsset,
        storage: Storage,
    ) -> Result<AssetVersion> {
        if id.is_none() {
            return Err(invalid("the empty asset ID names no asset"));
        }
        let decoded_len = cooked.bytes.len() as u64;
        if decoded_len > MAX_READ_BYTES {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!("{decoded_len} cooked bytes are more than a store reads in one request"),
            ));
        }
        let version = cooked.version();
        if let Some(entry) = self.entries.get(&id) {
            if entry.version == version {
                return Ok(version);
            }
            return Err(invalid(format!(
                "asset {:#018x} was added with version {:#018x} and again with {:#018x}",
                id.to_bits(),
                entry.version.to_bits(),
                version.to_bits()
            )));
        }
        let frame = match storage {
            Storage::Raw => None,
            Storage::Zstd { level } => compress(&cooked.bytes, level)?,
        };
        let stored: &[u8] = frame.as_deref().unwrap_or(&cooked.bytes);
        self.payloads
            .try_reserve(stored.len())
            .map_err(|_| Error::new(ErrorKind::ResourceExhausted, "the payloads cannot grow"))?;
        let offset = self.payloads.len() as u64;
        self.payloads.extend_from_slice(stored);
        let entry = Entry {
            id,
            version,
            header: cooked.header.clone(),
            compressed: frame.is_some(),
            offset,
            stored_len: stored.len() as u64,
            decoded_len,
        };
        self.entries.insert(id, entry);
        Ok(version)
    }

    /// The pack's bytes: the payloads, the index with its entries in ascending
    /// [`AssetId`] order, and the trailer.
    ///
    /// The whole pack is held in memory, as every output the cooker writes at milestone 1
    /// is (`crates/io/README.md`, "Not in this crate yet"). Fails with
    /// `ErrorKind::ResourceExhausted` if the index would be longer than
    /// `lactose_io::MAX_READ_BYTES`, which no mount could read.
    pub fn finish(self) -> Result<Vec<u8>> {
        let index_len = self.entries.values().try_fold(4u64, |len, entry| {
            let dependencies = entry.header.dependencies().len() as u64;
            len.checked_add(dependencies.checked_mul(8)?.checked_add(ENTRY_LEN)?)
        });
        let index_len = index_len.filter(|&len| len <= MAX_READ_BYTES).ok_or_else(|| {
            Error::new(
                ErrorKind::ResourceExhausted,
                "the index would be longer than a mount reads in one request",
            )
        })?;
        let exhausted = |_| Error::new(ErrorKind::ResourceExhausted, "the pack cannot grow");
        let mut index = Vec::new();
        index.try_reserve_exact(index_len as usize).map_err(exhausted)?;
        // Both counts are below `u32::MAX`: the index is at most `MAX_READ_BYTES` long.
        let count = |len: usize| u32::try_from(len).expect("bounded by the index's length");
        index.extend_from_slice(&count(self.entries.len()).to_le_bytes());
        for entry in self.entries.values() {
            let header = &entry.header;
            index.extend_from_slice(&entry.id.to_bits().to_le_bytes());
            index.extend_from_slice(&entry.version.to_bits().to_le_bytes());
            index.extend_from_slice(&header.kind().0.to_le_bytes());
            index.extend_from_slice(&header.format_version().to_le_bytes());
            index.extend_from_slice(&count(header.dependencies().len()).to_le_bytes());
            for dependency in header.dependencies() {
                index.extend_from_slice(&dependency.to_bits().to_le_bytes());
            }
            index.push(if entry.compressed { ZSTD } else { RAW });
            index.extend_from_slice(&entry.offset.to_le_bytes());
            index.extend_from_slice(&entry.stored_len.to_le_bytes());
            index.extend_from_slice(&entry.decoded_len.to_le_bytes());
        }
        debug_assert_eq!(index.len() as u64, index_len);

        let mut pack = self.payloads;
        pack.try_reserve_exact(index.len() + TRAILER_LEN as usize).map_err(exhausted)?;
        pack.extend_from_slice(&index);
        pack.extend_from_slice(&index_len.to_le_bytes());
        pack.extend_from_slice(&xxh64(&index, 0).to_le_bytes());
        pack.extend_from_slice(&MAGIC);
        pack.extend_from_slice(&PACK_FORMAT_VERSION.to_le_bytes());
        pack.extend_from_slice(&0u16.to_le_bytes());
        Ok(pack)
    }
}

impl Default for PackBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// One zstd frame of `bytes` at `level`, or `None` where the frame would be no shorter
/// than the bytes. The frame carries its content size, and its window is the level's:
/// zstd's tables cap it at 2^27 for every level from 1 to 22 and shrink it to the content,
/// so the frame meets each rule `lactose_io` checks of a stored range.
fn compress(bytes: &[u8], level: i32) -> Result<Option<Vec<u8>>> {
    if !(1..=22).contains(&level) {
        return Err(invalid(format!("zstd level {level} is outside 1 to 22")));
    }
    if bytes.is_empty() {
        // No frame is shorter than nothing.
        return Ok(None);
    }
    let exhausted = |error: std::io::Error| {
        Error::new(ErrorKind::ResourceExhausted, format!("zstd could not compress: {error}"))
    };
    let mut compressor = zstd::bulk::Compressor::new(level).map_err(exhausted)?;
    compressor.include_contentsize(true).map_err(exhausted)?;
    let frame = compressor.compress(bytes).map_err(exhausted)?;
    Ok((frame.len() < bytes.len()).then_some(frame))
}

/// What a mount learns from the last [`TRAILER_LEN`] bytes of a pack.
pub(crate) struct Trailer {
    /// N, the index's length: within the file, and readable in one request.
    pub(crate) index_len: u64,
    pub(crate) index_hash: u64,
}

impl Trailer {
    /// Checks the trailer of a file of `file_len` bytes in `README.md`'s order: the magic
    /// and the format version before anything else, then the reserved word, then N
    /// against the file and only then against the largest read.
    pub(crate) fn parse(bytes: &[u8], file_len: u64) -> Result<Self> {
        let mut reader = Reader { bytes };
        let index_len = reader.u64()?;
        let index_hash = reader.u64()?;
        let magic: [u8; 4] = reader.array()?;
        let version = u16::from_le_bytes(reader.array()?);
        let reserved = u16::from_le_bytes(reader.array()?);
        if magic != MAGIC {
            return Err(invalid("the file does not end in a pack's magic"));
        }
        if version != PACK_FORMAT_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!("a pack of format {version}; this build reads {PACK_FORMAT_VERSION}"),
            ));
        }
        if reserved != 0 {
            return Err(invalid("the trailer's reserved word is not zero"));
        }
        if index_len > file_len.saturating_sub(TRAILER_LEN) {
            return Err(invalid(format!(
                "an index of {index_len} bytes does not fit in a file of {file_len}"
            )));
        }
        if index_len > MAX_READ_BYTES {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!("an index of {index_len} bytes is more than a mount reads in one request"),
            ));
        }
        Ok(Self { index_len, index_hash })
    }
}

/// Decodes and checks the index a [`Trailer`] named, in a file whose payload area is its
/// first `payload_len` bytes: every rule of `README.md`, "The pack format", that needs no
/// payload. Every count is checked against the bytes that remain before anything is
/// allocated for it.
pub(crate) fn decode_index(index: &[u8], hash: u64, payload_len: u64) -> Result<Vec<Entry>> {
    if xxh64(index, 0) != hash {
        return Err(invalid("the index does not hash to the trailer's hash"));
    }
    let mut reader = Reader { bytes: index };
    let count = u64::from(reader.u32()?);
    if count.saturating_mul(ENTRY_LEN) > reader.remaining() {
        return Err(invalid(format!("{count} entries do not fit in the index")));
    }
    let mut entries = Vec::with_capacity(count as usize);
    let mut previous = AssetId::NONE;
    for _ in 0..count {
        let id = AssetId::from_bits(reader.u64()?);
        if id <= previous {
            return Err(invalid("asset IDs are not non-zero and strictly ascending"));
        }
        previous = id;
        let version = AssetVersion::from_bits(reader.u64()?)
            .ok_or_else(|| invalid("an entry's version is zero"))?;
        let kind = AssetKind(reader.u64()?);
        let format_version = reader.u32()?;
        let dependency_count = u64::from(reader.u32()?);
        if dependency_count.saturating_mul(8).saturating_add(ENTRY_TAIL_LEN) > reader.remaining() {
            return Err(invalid(format!(
                "{dependency_count} dependencies do not fit in the index"
            )));
        }
        let mut dependencies = Vec::with_capacity(dependency_count as usize);
        let mut previous = AssetId::NONE;
        for _ in 0..dependency_count {
            let dependency = AssetId::from_bits(reader.u64()?);
            if dependency <= previous {
                return Err(invalid("dependencies are not non-zero and strictly ascending"));
            }
            previous = dependency;
            dependencies.push(dependency);
        }
        let compressed = match reader.u8()? {
            RAW => false,
            ZSTD => true,
            code => return Err(invalid(format!("storage code {code} is neither raw nor zstd"))),
        };
        let offset = reader.u64()?;
        let stored_len = reader.u64()?;
        let decoded_len = reader.u64()?;
        if offset.checked_add(stored_len).is_none_or(|end| end > payload_len) {
            return Err(invalid("a stored range ends outside the payload area"));
        }
        if stored_len > MAX_READ_BYTES || decoded_len > MAX_READ_BYTES {
            return Err(invalid("a stored or decoded length is longer than one read"));
        }
        if !compressed && decoded_len != stored_len {
            return Err(invalid("a raw entry's decoded length is not its stored length"));
        }
        if compressed && stored_len == 0 {
            return Err(invalid("a zstd entry stores no bytes"));
        }
        let header = CookedHeader::new(kind, format_version, dependencies);
        entries.push(Entry { id, version, header, compressed, offset, stored_len, decoded_len });
    }
    if reader.remaining() != 0 {
        return Err(invalid("bytes are left over after the index's last entry"));
    }
    Ok(entries)
}

fn invalid(message: impl Into<Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Little-endian reads that fail as values at the end of the bytes.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn remaining(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let (head, rest) = self
            .bytes
            .split_first_chunk()
            .ok_or_else(|| invalid("the index ends inside an entry"))?;
        self.bytes = rest;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
}
