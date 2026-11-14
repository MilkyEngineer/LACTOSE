//! The pack: the file the cooker writes and the store mounts.
//!
//! A pack is payloads, then an index, then a 24-byte trailer, every integer little-endian.
//! `README.md`, "The pack format", is the layout byte for byte and every rule a mount
//! checks; this module is its one writer, [`PackBuilder`], and (hidden) its one reader, the
//! index decode a mount runs. Nothing here touches a file: a pack is built as bytes and
//! written by its caller through `lactose_io::Io::replace`.

use lactose_asset_api::{AssetId, AssetVersion, CookedAsset, CookedHeader};
use lactose_core::Result;

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
struct Entry {
    id: AssetId,
    version: AssetVersion,
    header: CookedHeader,
    /// Whether the payload is one zstd frame; raw otherwise.
    compressed: bool,
    offset: u64,
    stored_len: u64,
    decoded_len: u64,
}

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
    /// One entry per add. Hidden and changeable.
    entries: Vec<Entry>,
}

impl PackBuilder {
    /// An empty pack. An empty pack is a valid pack: it mounts and names nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self { payloads: Vec::new(), entries: Vec::new() }
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
        todo!("append the payload and its index entry")
    }

    /// The pack's bytes: the payloads, the index with its entries in ascending
    /// [`AssetId`] order, and the trailer.
    ///
    /// The whole pack is held in memory, as every output the cooker writes at milestone 1
    /// is (`crates/io/README.md`, "Not in this crate yet"). Fails with
    /// `ErrorKind::ResourceExhausted` if the index would be longer than
    /// `lactose_io::MAX_READ_BYTES`, which no mount could read.
    pub fn finish(self) -> Result<Vec<u8>> {
        todo!("write the index and the trailer after the payloads")
    }
}

impl Default for PackBuilder {
    fn default() -> Self {
        Self::new()
    }
}
