//! The save container (architecture sections 15.3, 15.4 and 19): a self-describing binary
//! document with an engine version, a schema hash, a per-region index and one section of
//! [`RegionRecords`] per region, each section encoded, compressed and hashed on its own so
//! a region is written on a worker and read without the rest. CARTON is its name on the
//! tools that read it.
//!
//! This facade fixes the shape (the types here) and the invariants of the encoding
//! (`README.md`, "The container"); the byte layout within them is the implementation's,
//! pinned by golden bytes once written.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use lactose_core::{Result, Tick};
use lactose_ecs::RegionId;
use lactose_sim::SavePoint;

use crate::diff::RegionRecords;
use crate::schema::{SchemaHash, SchemaTable};

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
        todo!("check, write the section's content, compress and hash it")
    }
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
        todo!("encode each section and assemble")
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
        todo!("sort, place, write the block and the sections")
    }

    /// How many bytes from the front of the file [`index`](Self::index) needs, from the
    /// first [`PREAMBLE_BYTES`] of it: the preamble plus the header-and-index block. Fails
    /// with `InvalidData` for fewer than [`PREAMBLE_BYTES`] bytes, a wrong magic, a
    /// non-zero reserved word or a block longer than [`MAX_CONTAINER_BYTES`], and with
    /// `VersionMismatch` for another format version. Reads nothing else, so a reader asks
    /// `io` for the preamble, then for the rest of the prefix.
    pub fn index_len(preamble: &[u8]) -> Result<usize> {
        todo!("check the preamble and read the block length")
    }

    /// The header and the index from the front of the file: `prefix` is at least
    /// [`index_len`](Self::index_len) bytes of it (more is ignored; a whole file will do),
    /// and nothing beyond the block is read. Fails as `index_len` fails, and with
    /// `InvalidData` for a block whose hash does not match, an index that is not strictly
    /// ascending by region, an entry whose section lies within the block or beyond
    /// [`MAX_CONTAINER_BYTES`] or overlaps another's, or an entry whose decompressed size
    /// exceeds [`MAX_SECTION_BYTES`].
    pub fn index(prefix: &[u8]) -> Result<ContainerIndex> {
        todo!("check the block's hash, read the header and the entries")
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
        todo!("check the hash, decompress to the stated size, decode the content")
    }

    /// One region's records from a whole file held in memory: [`index`](Self::index), the
    /// entry (`NotFound` for a region the index lacks), then
    /// [`decode_section`](Self::decode_section) over the entry's bytes (`InvalidData` if
    /// `bytes` is too short for them).
    pub fn decode_region(bytes: &[u8], region: RegionId) -> Result<RegionRecords> {
        todo!("index, find the entry, decode its section")
    }

    /// Every region's records from a whole file held in memory: [`index`](Self::index),
    /// then [`decode_section`](Self::decode_section) for every entry. Fails as they fail,
    /// on the first section that does: a caller that wants what is readable of a damaged
    /// file reads the index and each section itself (`README.md`, "The container").
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        todo!("index, then every section")
    }
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
