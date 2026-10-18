//! The save container (architecture sections 15.3 and 15.4): a self-describing binary
//! document with an engine version, a schema hash, a per-region index and one section of
//! [`RegionRecords`] per region. CARTON is its name on the tools that read it.
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

/// The largest encoded container, in bytes: 1 GiB. A reader refuses anything longer before
/// reading it, and a writer refuses to produce what no reader would accept.
pub const MAX_CONTAINER_BYTES: usize = 1024 * 1024 * 1024;

/// The largest region section once decompressed, in bytes: 512 MiB, room for an overlay at
/// `lactose_bundle::MAX_DOCUMENT_BYTES` and the resources beside it. The index states each
/// section's decompressed size and a reader refuses a larger one before decompressing, so
/// what decoding an untrusted container can cost is bounded by the index it reads first.
pub const MAX_SECTION_BYTES: usize = 512 * 1024 * 1024;

/// The engine that wrote a container, from the workspace version. Recorded for tools and
/// diagnostics; a load never refuses on it, since the schema hash and each document's own
/// schema table decide whether the content can be read.
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
    /// every document inside matches its schemas and no migration runs.
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
/// without the section being decompressed, for a tool that lists a save and for a load
/// that takes regions one at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RegionIndexEntry {
    /// The region.
    pub region: RegionId,
    /// Its seed, as the section carries it.
    pub seed: u64,
    /// The content hash of the baseline its overlay was made against.
    pub baseline_hash: u64,
    /// The overlay's digest, or `None` for a region in its cooked state.
    pub overlay_digest: Option<u64>,
    /// The section's size once decompressed, at most [`MAX_SECTION_BYTES`].
    pub bytes: u64,
    /// The section's size in the container.
    pub compressed_bytes: u64,
}

/// A container's header and index, read without decompressing any section.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContainerIndex {
    /// The header.
    pub header: ContainerHeader,
    /// One entry per region, strictly ascending by region.
    pub regions: Vec<RegionIndexEntry>,
}

/// A save: a header and the records of every region it holds, one section per region.
///
/// Built by `app` from a `SavePoint`: [`ContainerHeader::of`] the save point, then
/// [`insert`](Self::insert) of the [`RegionRecords`] of each export, diffed on background
/// workers; encoded to bytes with [`encode`](Self::encode), which `app` writes through
/// `io` to a temporary file, syncs and renames over the slot (architecture section 15.3).
/// Read back whole with [`decode`](Self::decode), or by its [`index`](Self::index) and one
/// region at a time with [`decode_region`](Self::decode_region).
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
    /// section the container held for it.
    pub fn insert(&mut self, records: RegionRecords) -> Option<RegionRecords> {
        self.regions.insert(records.region, records)
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

    /// The container's bytes: the header, the index, and one section per region, each
    /// compressed on its own with zstd and checked by its own hash, under one trailing
    /// hash; the invariants are in `README.md` ("The container"). Deterministic for one
    /// build: equal containers encode to equal bytes. Fails with `ResourceExhausted` if a
    /// section would exceed [`MAX_SECTION_BYTES`] or the whole [`MAX_CONTAINER_BYTES`],
    /// and with `InvalidData` if a section's resources and resource schemas disagree.
    pub fn encode(&self) -> Result<Vec<u8>> {
        todo!("write the header, the index and each compressed, hashed section")
    }

    /// Decodes and validates untrusted bytes, every section included: [`index`](Self::index)
    /// then [`decode_region`](Self::decode_region) for each entry. Fails as they fail.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        todo!("read the index, then every section")
    }

    /// Reads the header and the index of untrusted bytes without decompressing a section.
    ///
    /// Fails with `ResourceExhausted` for more than [`MAX_CONTAINER_BYTES`], with
    /// `VersionMismatch` for another format version, and with `InvalidData` for anything
    /// that is not a container: a wrong magic, a damaged trailing hash, an index that is
    /// not strictly ascending by region or whose entries do not fit the bytes, or an entry
    /// whose decompressed size exceeds [`MAX_SECTION_BYTES`].
    pub fn index(bytes: &[u8]) -> Result<ContainerIndex> {
        todo!("check the frame, read the header and the index")
    }

    /// Decodes one region's section of untrusted bytes: the index, then that section
    /// alone. Fails as [`index`](Self::index) fails, with `NotFound` for a region the index
    /// lacks, and with `InvalidData` for a section whose hash does not match, that does not
    /// decompress to exactly the size its entry states, or whose documents are not the
    /// canonical encodings `lactose_bundle` accepts.
    pub fn decode_region(bytes: &[u8], region: RegionId) -> Result<RegionRecords> {
        todo!("read the index, find the entry, decompress and decode its section")
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
