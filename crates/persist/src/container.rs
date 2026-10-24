//! The save container (architecture sections 15.3, 15.4 and 19): a self-describing binary
//! document with an engine version, a schema hash, a per-region index and one section of
//! [`RegionRecords`] per region, each section encoded, compressed and hashed on its own so
//! a region is written on a worker and read without the rest. CARTON is its name on the
//! tools that read it.
//!
//! The shape is the types here and the invariants of the encoding are `README.md`'s ("The
//! container"); the byte layout within them is this file's, pinned by the golden bytes of
//! `tests/format.rs`:
//!
//! - the preamble: the magic `LSAV`, the format (`u16`), a zero `u16`, the block's length
//!   (`u64`);
//! - the block: the engine version (three `u16`), the schema hash (`u64`), the tick
//!   (`u32`), the `NetId` counter (`u64`, never zero), the entry count (`u32`), each entry
//!   as region, seed and baseline hash (`u64` each), the overlay's presence (`u8`, zero or
//!   one) and digest (`u64`, zero when absent), offset, compressed size, decompressed size
//!   and hash (`u64` each), then the xxh64 of the block's preceding bytes;
//! - a section's content, before compression: region, seed and baseline hash (`u64`
//!   each), the overlay's presence (`u8`) and, when present, its length (`u64`) and
//!   `Overlay::encode`; the resource schema table as `lactose-bundle` writes one; each
//!   resource as its key (`u64`) and the bytes of its schema's size, in the table's order;
//!   the transient keys (a count and a `u64` each, ascending); the layout digests (a count
//!   and a key and digest, `u64` each, ascending). One zstd frame at `SECTION_LEVEL`, with
//!   its content size and without a checksum (the entry's xxh64 is the check).

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use lactose_bundle::{ComponentData, Overlay};
use lactose_core::{Error, ErrorKind, Result, Tick, xxh64};
use lactose_ecs::RegionId;
use lactose_sim::SavePoint;
use zstd::zstd_safe::CParameter;

use crate::codec::{Reader, Writer, invalid};
use crate::diff::RegionRecords;
use crate::schema::{LayoutDigest, SchemaHash, SchemaTable};

/// The container format this build writes and reads. A reader refuses any other with
/// `VersionMismatch`: before the first shipped save the format changes freely with this
/// number, and after it a change ships with a converter.
pub const CONTAINER_FORMAT_VERSION: u16 = 1;

/// The first bytes of a container, which say how long the header-and-index block is:
/// what a reader reads first, then [`Container::index_len`] more, before it knows
/// anything else. The magic, the format version, a zero `u16` and the block's length as a
/// `u64`.
pub const PREAMBLE_BYTES: usize = 16;

/// The largest encoded container, in bytes: 1 GiB. A reader refuses an index that places a
/// section beyond it, and a writer refuses to produce what no reader would accept.
pub const MAX_CONTAINER_BYTES: usize = 1024 * 1024 * 1024;

/// The largest region section once decompressed, in bytes: 512 MiB, room for an overlay at
/// `lactose_bundle::MAX_DOCUMENT_BYTES` and the resources beside it. The index states each
/// section's decompressed size and a reader refuses a larger one before decompressing, so
/// what decoding an untrusted section can cost is bounded by the index it reads first.
pub const MAX_SECTION_BYTES: usize = 512 * 1024 * 1024;

/// The engine that wrote a container, from the workspace version. Recorded for tools and
/// diagnostics; a load never refuses on it, since the schema hash and each section's own
/// schemas and digests decide whether the content can be read.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EngineVersion {
    /// The major version.
    pub major: u16,
    /// The minor version.
    pub minor: u16,
    /// The patch version.
    pub patch: u16,
}

impl EngineVersion {
    /// This build's version: the workspace's `version` at compile time.
    pub const CURRENT: Self = Self {
        major: parse_u16(env!("CARGO_PKG_VERSION_MAJOR")),
        minor: parse_u16(env!("CARGO_PKG_VERSION_MINOR")),
        patch: parse_u16(env!("CARGO_PKG_VERSION_PATCH")),
    };
}

/// A decimal version component, at compile time. Cargo hands over plain digits.
const fn parse_u16(digits: &str) -> u16 {
    let bytes = digits.as_bytes();
    let mut value: u16 = 0;
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i].is_ascii_digit(), "a version component is decimal digits");
        value = value * 10 + (bytes[i] - b'0') as u16;
        i += 1;
    }
    value
}

/// What a container states about itself and about the simulation it was taken from: the
/// part of a save that is not a region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ContainerHeader {
    /// The container format, [`CONTAINER_FORMAT_VERSION`] when written by this build.
    pub format: u16,
    /// The engine that wrote it.
    pub engine: EngineVersion,
    /// [`SchemaHash::of`] the writing build's registry: equal to the loading build's means
    /// every type every section carries is at the registry's version and layout and no
    /// migration runs.
    pub schema_hash: SchemaHash,
    /// The tick the save point was taken at: what `SimulationBuilder::tick` takes on a
    /// load.
    pub tick: Tick,
    /// The runtime `NetId` counter at the save point: what `SimulationBuilder::next_net_id`
    /// takes, so RNG streams and ordering survive a load (architecture section 15.5).
    pub next_net_id: NonZeroU64,
}

impl ContainerHeader {
    /// A header written by this build for a save point at `tick` with `next_net_id`, over
    /// the registry whose saved schemas hash to `schema_hash`.
    #[must_use]
    pub fn new(tick: Tick, next_net_id: NonZeroU64, schema_hash: SchemaHash) -> Self {
        Self {
            format: CONTAINER_FORMAT_VERSION,
            engine: EngineVersion::CURRENT,
            schema_hash,
            tick,
            next_net_id,
        }
    }

    /// The header of a save of `save`, taken over the registry `schemas` was built from.
    #[must_use]
    pub fn of(save: &SavePoint, schemas: &SchemaTable) -> Self {
        Self::new(save.tick, save.next_net_id, schemas.hash())
    }
}

/// One entry of the per-region index: what a container states about a region's section
/// without the section being read, for a tool that lists a save and for a load that takes
/// regions one at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RegionIndexEntry {
    /// The region, as the section's content also states it.
    pub region: RegionId,
    /// Its seed, as the section's content also states it.
    pub seed: u64,
    /// The content hash of the baseline its overlay was made against, as the section's
    /// content also states it.
    pub baseline_hash: u64,
    /// The overlay's digest, or `None` for a region in its cooked state; what the
    /// section's overlay must have.
    pub overlay_digest: Option<u64>,
    /// Where the section's bytes start, from the first byte of the container. Zero in an
    /// [`EncodedSection`] until [`Container::assemble`] places it.
    pub offset: u64,
    /// The section's size in the container: how many bytes to read at `offset`.
    pub compressed_bytes: u64,
    /// The section's size once decompressed, at most [`MAX_SECTION_BYTES`].
    pub bytes: u64,
    /// The xxh64, seed zero, of the section's `compressed_bytes` bytes: what
    /// [`Container::decode_section`] checks before it decompresses anything.
    pub hash: u64,
}

/// A container's header and index, read from the front of the file without any section.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContainerIndex {
    /// The header.
    pub header: ContainerHeader,
    /// One entry per region, strictly ascending by region.
    pub regions: Vec<RegionIndexEntry>,
}

impl ContainerIndex {
    /// The entry of `region`, by binary search.
    #[must_use]
    pub fn entry(&self, region: RegionId) -> Option<&RegionIndexEntry> {
        let at = self.regions.binary_search_by_key(&region, |entry| entry.region).ok()?;
        Some(&self.regions[at])
    }
}

/// One region's section as it goes into a container: its index entry (with `offset` zero)
/// and its compressed bytes. Only [`RegionRecords::encode_section`] builds one, on
/// whichever worker diffed the region, so an entry agrees with its bytes by construction
/// and [`Container::assemble`] takes them as made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncodedSection {
    entry: RegionIndexEntry,
    bytes: Vec<u8>,
}

impl EncodedSection {
    /// The entry, as [`Container::index`] will report it once the section is placed:
    /// `offset` zero, the sizes and the hash those of [`bytes`](Self::bytes).
    #[must_use]
    pub fn entry(&self) -> &RegionIndexEntry {
        &self.entry
    }

    /// The section's bytes, `entry().compressed_bytes` long, hashing to `entry().hash`.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl RegionRecords {
    /// These records as a section: one zstd frame, level and parameters pinned by the
    /// implementation, carrying its content size, whose content is the region, the seed
    /// and the baseline hash (so a section states what its entry claims), the overlay as
    /// `Overlay::encode` behind a presence byte, the resource schema table as
    /// `lactose-bundle` writes one, the resources as key and bytes of the schema's size,
    /// the transient keys ascending, and the layout digests as key and digest ascending;
    /// with its entry filled from the records and the bytes, `offset` zero. Fails with
    /// `InvalidData` for records that fail [`check`](Self::check), and with
    /// `ResourceExhausted` if the content would exceed [`MAX_SECTION_BYTES`]. Pure and
    /// independent per region, so `app` runs it beside `from_export` on a background
    /// worker.
    pub fn encode_section(&self) -> Result<EncodedSection> {
        self.check()?;
        let content = self.content()?;
        let compress = || -> std::io::Result<Vec<u8>> {
            let mut compressor = zstd::bulk::Compressor::new(SECTION_LEVEL)?;
            compressor.set_parameter(CParameter::ChecksumFlag(false))?;
            compressor.set_parameter(CParameter::ContentSizeFlag(true))?;
            compressor.compress(&content)
        };
        let mut bytes = compress().map_err(|error| {
            Error::new(ErrorKind::Io, format!("compressing a section failed: {error}"))
        })?;
        bytes.shrink_to_fit();
        let entry = RegionIndexEntry {
            region: self.region,
            seed: self.seed,
            baseline_hash: self.baseline_hash,
            overlay_digest: self.overlay.as_ref().map(|overlay| overlay.digest()),
            offset: 0,
            compressed_bytes: bytes.len() as u64,
            bytes: content.len() as u64,
            hash: xxh64(&bytes, 0),
        };
        Ok(EncodedSection { entry, bytes })
    }

    /// The section's content before compression, as [`encode_section`](Self::encode_section)
    /// states it.
    fn content(&self) -> Result<Vec<u8>> {
        let mut out = Writer::default();
        out.u64(self.region.0);
        out.u64(self.seed);
        out.u64(self.baseline_hash);
        match &self.overlay {
            Some(overlay) => {
                let encoded = overlay.encode();
                out.u8(1);
                out.u64(encoded.len() as u64);
                out.bytes(&encoded);
            }
            None => out.u8(0),
        }
        out.schemas(&self.resource_schemas);
        for data in &self.resources {
            out.u64(data.component.0);
            out.bytes(&data.bytes);
        }
        out.count(self.transient_resources.len());
        for component in &self.transient_resources {
            out.u64(component.0);
        }
        out.count(self.layouts.len());
        for (component, layout) in &self.layouts {
            out.u64(component.0);
            out.u64(layout.0);
        }
        if out.bytes.len() > MAX_SECTION_BYTES {
            return Err(Error::new(
                ErrorKind::ResourceExhausted,
                format!("a section of {} bytes is over MAX_SECTION_BYTES", out.bytes.len()),
            ));
        }
        Ok(out.bytes)
    }

    /// The records a section's content states, as [`content`](Self::content) writes them,
    /// canonical and nothing after them.
    fn from_content(content: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(content);
        let region = RegionId(reader.u64()?);
        let seed = reader.u64()?;
        let baseline_hash = reader.u64()?;
        let overlay = if reader.flag()? {
            let len = reader.len()?;
            Some(Arc::new(Overlay::decode(reader.take(len)?)?))
        } else {
            None
        };
        let resource_schemas = reader.schemas()?;
        let mut resources = Vec::with_capacity(resource_schemas.len());
        for schema in &resource_schemas {
            let component = reader.component()?;
            if component != schema.component() {
                return Err(invalid("a resource is not the type its schema names"));
            }
            let bytes = reader.take(schema.element_size() as usize)?.to_vec();
            resources.push(ComponentData { component, bytes });
        }
        let count = reader.count(8)?;
        let mut transient_resources = Vec::with_capacity(count);
        for _ in 0..count {
            transient_resources.push(reader.component()?);
        }
        let count = reader.count(16)?;
        let mut layouts = BTreeMap::new();
        for _ in 0..count {
            let (component, layout) = (reader.component()?, LayoutDigest(reader.u64()?));
            if layouts.last_key_value().is_some_and(|(&last, _)| last >= component) {
                return Err(invalid("the layout digests are not strictly ascending"));
            }
            layouts.insert(component, layout);
        }
        reader.finish()?;
        let records = Self {
            region,
            seed,
            baseline_hash,
            overlay,
            resources,
            resource_schemas,
            transient_resources,
            layouts,
        };
        records.check()?;
        Ok(records)
    }
}

/// The zstd level of every section: the measured choice (`docs/verification.md`, D129's
/// save-cost row). Pinned: a change changes the bytes `tests/format.rs` holds.
const SECTION_LEVEL: i32 = 3;

/// The container's magic, the first four bytes of the preamble.
const MAGIC: [u8; 4] = *b"LSAV";

/// The header's bytes in the block: the engine version, the schema hash, the tick, the
/// counter and the entry count.
const HEADER_BYTES: usize = 6 + 8 + 4 + 8 + 4;

/// One index entry's bytes: region, seed, baseline hash, the overlay's presence byte and
/// digest, offset, both sizes and the hash.
const ENTRY_BYTES: usize = 8 * 3 + 1 + 8 + 8 * 4;

/// The block's trailing hash.
const HASH_BYTES: usize = 8;

fn too_large(what: &str, len: usize, cap: usize) -> Error {
    Error::new(ErrorKind::ResourceExhausted, format!("{what} of {len} bytes is over {cap}"))
}

/// A save: a header and the records of every region it holds, one section per region.
///
/// Built by `app` from a `SavePoint`: [`ContainerHeader::of`] the save point, then the
/// [`RegionRecords`] of each export, diffed and encoded on background workers, and
/// [`assemble`](Self::assemble)d into the bytes `app` writes through `io` to a temporary
/// file, syncs and renames over the slot (architecture section 15.3); or, held in memory,
/// [`insert`](Self::insert)ed and [`encode`](Self::encode)d in one call. Read back by its
/// [`index`](Self::index) and one [`decode_section`](Self::decode_section) per region the
/// caller wants, which is how a streaming load and the corrupt-save path of section 19 read
/// it; [`decode`](Self::decode) and [`decode_region`](Self::decode_region) are the
/// in-memory shorthands over a whole file.
///
/// Two containers compare by content. `decode(encode(c))` equals `c`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Container {
    header: ContainerHeader,
    regions: BTreeMap<RegionId, RegionRecords>,
}

impl Container {
    /// An empty container under `header`.
    #[must_use]
    pub fn new(header: ContainerHeader) -> Self {
        Self { header, regions: BTreeMap::new() }
    }

    /// The header.
    #[must_use]
    pub fn header(&self) -> &ContainerHeader {
        &self.header
    }

    /// Adds `records` as the section of their region, replacing and returning the
    /// section the container held for it. Fails with `InvalidData`, changing nothing, for
    /// records that fail [`RegionRecords::check`].
    pub fn insert(&mut self, records: RegionRecords) -> Result<Option<RegionRecords>> {
        records.check()?;
        Ok(self.regions.insert(records.region, records))
    }

    /// Removes and returns the section of `region`.
    pub fn remove(&mut self, region: RegionId) -> Option<RegionRecords> {
        self.regions.remove(&region)
    }

    /// The section of `region`.
    #[must_use]
    pub fn region(&self, region: RegionId) -> Option<&RegionRecords> {
        self.regions.get(&region)
    }

    /// Every section, ascending by region.
    #[must_use]
    pub fn regions(&self) -> Sections<'_> {
        Sections { values: self.regions.values() }
    }

    /// The number of regions the container holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// Whether the container holds no region.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// The container's bytes: [`encode_section`](RegionRecords::encode_section) of every
    /// section, then [`assemble`](Self::assemble). Fails as they fail.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let sections = self.regions().map(RegionRecords::encode_section).collect::<Result<_>>()?;
        Self::assemble(&self.header, sections)
    }

    /// The bytes of a container holding `sections` under `header`: the preamble, then the
    /// header-and-index block (the header, one entry per section ascending by region with
    /// its `offset` assigned, and the xxh64, seed zero, of the block's preceding bytes),
    /// then the sections' bytes back to back in index order. There is no whole-file hash:
    /// the block and each section are checked on their own, so damage to one section
    /// leaves the others readable. Deterministic for one build: equal inputs give equal
    /// bytes. Fails with `InvalidData` if two sections name one region, and with
    /// `ResourceExhausted` if the whole would exceed [`MAX_CONTAINER_BYTES`]. The sections
    /// are taken as made, so `app` encodes each region on a worker and assembles here.
    pub fn assemble(header: &ContainerHeader, sections: Vec<EncodedSection>) -> Result<Vec<u8>> {
        if header.format != CONTAINER_FORMAT_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "this build writes container format {CONTAINER_FORMAT_VERSION}, not {}",
                    header.format
                ),
            ));
        }
        let mut sections = sections;
        sections.sort_by_key(|section| section.entry.region);
        if let Some(pair) =
            sections.windows(2).find(|pair| pair[0].entry.region == pair[1].entry.region)
        {
            return Err(invalid(format!("two sections of region {}", pair[0].entry.region.0)));
        }
        let block_len = HEADER_BYTES + sections.len() * ENTRY_BYTES + HASH_BYTES;
        let start = PREAMBLE_BYTES + block_len;
        let total = sections
            .iter()
            .try_fold(start, |end, section| end.checked_add(section.bytes.len()))
            .filter(|&total| total <= MAX_CONTAINER_BYTES)
            .ok_or_else(|| {
                let len = sections.iter().map(|section| section.bytes.len()).sum::<usize>();
                too_large("a container", start.saturating_add(len), MAX_CONTAINER_BYTES)
            })?;

        let mut out = Writer { bytes: Vec::with_capacity(total) };
        out.bytes(&MAGIC);
        out.u16(header.format);
        out.u16(0);
        out.u64(block_len as u64);
        out.u16(header.engine.major);
        out.u16(header.engine.minor);
        out.u16(header.engine.patch);
        out.u64(header.schema_hash.0);
        out.u32(header.tick.0);
        out.u64(header.next_net_id.get());
        out.count(sections.len());
        let mut offset = start as u64;
        for section in &sections {
            let entry = &section.entry;
            out.u64(entry.region.0);
            out.u64(entry.seed);
            out.u64(entry.baseline_hash);
            out.u8(u8::from(entry.overlay_digest.is_some()));
            out.u64(entry.overlay_digest.unwrap_or(0));
            out.u64(offset);
            out.u64(entry.compressed_bytes);
            out.u64(entry.bytes);
            out.u64(entry.hash);
            offset += entry.compressed_bytes;
        }
        let hash = xxh64(&out.bytes[PREAMBLE_BYTES..], 0);
        out.u64(hash);
        debug_assert_eq!(out.bytes.len(), start);
        for section in &sections {
            out.bytes(&section.bytes);
        }
        Ok(out.bytes)
    }

    /// How many bytes from the front of the file [`index`](Self::index) needs, from the
    /// first [`PREAMBLE_BYTES`] of it: the preamble plus the header-and-index block. Fails
    /// with `InvalidData` for fewer than [`PREAMBLE_BYTES`] bytes, a wrong magic, a
    /// non-zero reserved word or a block longer than [`MAX_CONTAINER_BYTES`], and with
    /// `VersionMismatch` for another format version. Reads nothing else, so a reader asks
    /// `io` for the preamble, then for the rest of the prefix.
    pub fn index_len(preamble: &[u8]) -> Result<usize> {
        let Some(preamble) = preamble.get(..PREAMBLE_BYTES) else {
            return Err(invalid(format!(
                "a container's preamble is {PREAMBLE_BYTES} bytes, got {}",
                preamble.len()
            )));
        };
        let mut reader = Reader::new(preamble);
        if reader.take(4)? != MAGIC {
            return Err(invalid("not a save container".to_owned()));
        }
        let format = reader.u16()?;
        if format != CONTAINER_FORMAT_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "container format {format} is not the supported format \
                     {CONTAINER_FORMAT_VERSION}"
                ),
            ));
        }
        if reader.u16()? != 0 {
            return Err(invalid("a container's reserved word is not zero".to_owned()));
        }
        let block_len = reader.u64()?;
        match usize::try_from(block_len) {
            Ok(len) if len <= MAX_CONTAINER_BYTES - PREAMBLE_BYTES => Ok(PREAMBLE_BYTES + len),
            _ => Err(invalid(format!(
                "a header-and-index block of {block_len} bytes is over MAX_CONTAINER_BYTES"
            ))),
        }
    }

    /// The header and the index from the front of the file: `prefix` is at least
    /// [`index_len`](Self::index_len) bytes of it (more is ignored; a whole file will do),
    /// and nothing beyond the block is read. Fails as `index_len` fails, and with
    /// `InvalidData` for a block whose hash does not match, an index that is not strictly
    /// ascending by region, an entry whose section lies within the block or beyond
    /// [`MAX_CONTAINER_BYTES`] or overlaps another's, or an entry whose decompressed size
    /// exceeds [`MAX_SECTION_BYTES`].
    pub fn index(prefix: &[u8]) -> Result<ContainerIndex> {
        let len = Self::index_len(prefix)?;
        let Some(block) = prefix.get(PREAMBLE_BYTES..len) else {
            return Err(invalid(format!(
                "the header and index are {len} bytes from the front, got {}",
                prefix.len()
            )));
        };
        let Some((sealed, hash)) = block.split_last_chunk::<HASH_BYTES>() else {
            return Err(invalid("the header-and-index block is shorter than its hash".to_owned()));
        };
        if u64::from_le_bytes(*hash) != xxh64(sealed, 0) {
            return Err(invalid("the header-and-index block's hash does not match".to_owned()));
        }
        let mut reader = Reader::new(sealed);
        let engine =
            EngineVersion { major: reader.u16()?, minor: reader.u16()?, patch: reader.u16()? };
        let schema_hash = SchemaHash(reader.u64()?);
        let tick = Tick(reader.u32()?);
        let next_net_id = NonZeroU64::new(reader.u64()?)
            .ok_or_else(|| invalid("a container's NetId counter is zero".to_owned()))?;
        let header = ContainerHeader {
            format: CONTAINER_FORMAT_VERSION,
            engine,
            schema_hash,
            tick,
            next_net_id,
        };
        let count = reader.count(ENTRY_BYTES)?;
        let mut regions: Vec<RegionIndexEntry> = Vec::with_capacity(count);
        for _ in 0..count {
            let region = RegionId(reader.u64()?);
            let (seed, baseline_hash) = (reader.u64()?, reader.u64()?);
            let present = reader.flag()?;
            let digest = reader.u64()?;
            if !present && digest != 0 {
                return Err(invalid("an absent overlay's digest is not zero".to_owned()));
            }
            let entry = RegionIndexEntry {
                region,
                seed,
                baseline_hash,
                overlay_digest: present.then_some(digest),
                offset: reader.u64()?,
                compressed_bytes: reader.u64()?,
                bytes: reader.u64()?,
                hash: reader.u64()?,
            };
            if regions.last().is_some_and(|last| last.region >= region) {
                return Err(invalid("the index is not strictly ascending by region".to_owned()));
            }
            let end = entry.offset.checked_add(entry.compressed_bytes);
            if entry.offset < len as u64 {
                return Err(invalid(format!(
                    "the section of region {} lies within the header-and-index block",
                    region.0
                )));
            }
            if end.is_none_or(|end| end > MAX_CONTAINER_BYTES as u64) {
                return Err(invalid(format!(
                    "the section of region {} lies beyond MAX_CONTAINER_BYTES",
                    region.0
                )));
            }
            if entry.bytes > MAX_SECTION_BYTES as u64 {
                return Err(invalid(format!(
                    "the section of region {} decompresses to {} bytes, over MAX_SECTION_BYTES",
                    region.0, entry.bytes
                )));
            }
            regions.push(entry);
        }
        reader.finish()?;
        let mut placed: Vec<&RegionIndexEntry> = regions.iter().collect();
        placed.sort_unstable_by_key(|entry| (entry.offset, entry.compressed_bytes));
        if let Some(pair) = placed
            .windows(2)
            .find(|pair| pair[0].offset + pair[0].compressed_bytes > pair[1].offset)
        {
            return Err(invalid(format!(
                "the sections of regions {} and {} overlap",
                pair[0].region.0, pair[1].region.0
            )));
        }
        Ok(ContainerIndex { header, regions })
    }

    /// One region's records from its section: `section` is exactly the
    /// `entry.compressed_bytes` bytes at `entry.offset` of the file, read by the caller.
    /// Fails with `InvalidData` for a slice of another length or whose xxh64 is not
    /// `entry.hash` (before decompressing), for a frame that does not decompress to exactly
    /// `entry.bytes`, for content that is not the canonical encodings `lactose-bundle` and
    /// this crate accept, for content whose region, seed, baseline hash, or overlay
    /// presence and digest differ from the entry's (an index that disagrees with its
    /// section is never shown as the section), or for records that fail
    /// [`RegionRecords::check`]; nothing is allocated beyond `entry.bytes` plus
    /// `lactose-bundle`'s bound on its documents. A failure here says nothing about any
    /// other section.
    pub fn decode_section(entry: &RegionIndexEntry, section: &[u8]) -> Result<RegionRecords> {
        if section.len() as u64 != entry.compressed_bytes {
            return Err(invalid(format!(
                "the section of region {} is {} bytes, its entry states {}",
                entry.region.0,
                section.len(),
                entry.compressed_bytes
            )));
        }
        if xxh64(section, 0) != entry.hash {
            return Err(invalid(format!(
                "the section of region {} does not hash to its entry's hash",
                entry.region.0
            )));
        }
        let content = decompress(entry, section)?;
        let records = RegionRecords::from_content(&content)?;
        let agrees = records.region == entry.region
            && records.seed == entry.seed
            && records.baseline_hash == entry.baseline_hash
            && records.overlay.as_ref().map(|overlay| overlay.digest()) == entry.overlay_digest;
        if !agrees {
            return Err(invalid(format!(
                "the section of region {} states other records than its entry",
                entry.region.0
            )));
        }
        Ok(records)
    }

    /// One region's records from a whole file held in memory: [`index`](Self::index), the
    /// entry (`NotFound` for a region the index lacks), then
    /// [`decode_section`](Self::decode_section) over the entry's bytes (`InvalidData` if
    /// `bytes` is too short for them).
    pub fn decode_region(bytes: &[u8], region: RegionId) -> Result<RegionRecords> {
        let index = Self::index(bytes)?;
        let entry = index.entry(region).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("the container holds no region {}", region.0))
        })?;
        Self::decode_section(entry, section_of(bytes, entry)?)
    }

    /// Every region's records from a whole file held in memory: [`index`](Self::index),
    /// then [`decode_section`](Self::decode_section) for every entry. Fails as they fail,
    /// on the first section that does: a caller that wants what is readable of a damaged
    /// file reads the index and each section itself (`README.md`, "The container").
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let index = Self::index(bytes)?;
        let mut regions = BTreeMap::new();
        for entry in &index.regions {
            regions.insert(entry.region, Self::decode_section(entry, section_of(bytes, entry)?)?);
        }
        Ok(Self { header: index.header, regions })
    }
}

/// The bytes of `entry`'s section within a whole file.
fn section_of<'a>(bytes: &'a [u8], entry: &RegionIndexEntry) -> Result<&'a [u8]> {
    let start = usize::try_from(entry.offset).ok();
    let end = start.zip(usize::try_from(entry.compressed_bytes).ok());
    end.and_then(|(start, len)| bytes.get(start..start.checked_add(len)?)).ok_or_else(|| {
        invalid(format!("the file ends before the section of region {}", entry.region.0))
    })
}

/// A section's content: exactly one zstd frame that states and decompresses to exactly
/// `entry.bytes`, at most [`MAX_SECTION_BYTES`] by the index's check. Nothing is
/// allocated beyond that before the frame is known to state it.
fn decompress(entry: &RegionIndexEntry, section: &[u8]) -> Result<Vec<u8>> {
    let refuse = |what: &str| invalid(format!("the section of region {} {what}", entry.region.0));
    let size = usize::try_from(entry.bytes)
        .ok()
        .filter(|&size| size <= MAX_SECTION_BYTES)
        .ok_or_else(|| refuse("is over MAX_SECTION_BYTES"))?;
    match zstd::zstd_safe::find_frame_compressed_size(section) {
        Ok(len) if len == section.len() => {}
        _ => return Err(refuse("is not exactly one zstd frame")),
    }
    match zstd::zstd_safe::get_frame_content_size(section) {
        Ok(Some(stated)) if stated == entry.bytes => {}
        _ => return Err(refuse("does not state its entry's decompressed size")),
    }
    let mut decompressor =
        zstd::bulk::Decompressor::new().map_err(|_| refuse("could not be decompressed"))?;
    let content = decompressor
        .decompress(section, size)
        .map_err(|_| refuse("does not decompress to its entry's size"))?;
    if content.len() != size {
        return Err(refuse("does not decompress to its entry's size"));
    }
    Ok(content)
}

/// The sections of a [`Container`], ascending by region. See [`Container::regions`].
#[derive(Clone, Debug)]
pub struct Sections<'a> {
    values: std::collections::btree_map::Values<'a, RegionId, RegionRecords>,
}

impl<'a> Iterator for Sections<'a> {
    type Item = &'a RegionRecords;

    fn next(&mut self) -> Option<Self::Item> {
        self.values.next()
    }
}

impl ExactSizeIterator for Sections<'_> {
    fn len(&self) -> usize {
        self.values.len()
    }
}
