//! The region baseline as an asset: the one asset kind this crate defines, because the
//! cooker, `app` and `lactose-persist` all name a baseline by its content hash and none of
//! them may define the kind for the others.

use std::sync::Arc;

use lactose_asset_api::{Asset, AssetCatalog, AssetId, AssetKind, CookedAsset, Loader, Readiness};
use lactose_bundle::Baseline;
use lactose_core::Result;

/// A region's cooked baseline as a published asset (architecture sections 14 and 15.2).
///
/// **The kind.** Its cooked bytes are `Baseline::encode`'s, the canonical document of
/// `lactose-bundle`, and it declares no dependencies: a region's asset set is data inside
/// the baseline (`Baseline::assets`), requested by whoever loads the region, so that a
/// baseline is readable (by a save's load, by the diff of an unload) without one mesh of
/// its region in memory.
///
/// **The identity.** A baseline is content-addressed: its [`AssetId`] is
/// [`RegionBaseline::id`] of its `Baseline::content_hash`, the hash a save's records, a
/// relevance message and a journal already name it by. So an ID has one version in every
/// store, "the baseline with content hash `h`" is an ordinary lookup of the newest version
/// of `RegionBaseline::id(h)`, and a re-cooked region is a new asset beside the old one,
/// which stays loadable for as long as a mounted pack holds it. Which baseline a region
/// currently has is not answered here (`README.md`, "Not in this crate yet").
///
/// It is not a `SimulationAsset`: no system reads a baseline through a `PinnedTable`. The
/// simulation receives it in a `RegionBundle`, which is why the asset holds an `Arc`.
#[derive(Clone, Debug)]
pub struct RegionBaseline {
    baseline: Arc<Baseline>,
}

impl RegionBaseline {
    /// The ID of the baseline whose `Baseline::content_hash` is `content_hash`:
    /// `AssetId::from_name` of the 32-byte name made of `region_baseline/` and
    /// `content_hash` as sixteen lower-case hexadecimal digits
    /// (`region_baseline/00000000000000ff` for 255).
    #[must_use]
    pub fn id(content_hash: u64) -> AssetId {
        todo!("hash the content-addressed name")
    }

    /// What the cooker adds to a pack for `baseline`: [`RegionBaseline::id`] of its content
    /// hash, and `baseline.encode()` as cooked bytes of this kind with no dependencies. The
    /// pair to pass to `PackBuilder::add`.
    #[must_use]
    pub fn cook(baseline: &Baseline) -> (AssetId, CookedAsset) {
        todo!("encode the baseline under its content-addressed ID")
    }

    /// The baseline, shared: what `RegionBundle::baseline` and a diff take.
    #[must_use]
    pub fn baseline(&self) -> &Arc<Baseline> {
        &self.baseline
    }

    /// The baseline whose content hash is `content_hash`, from `catalog`: the newest
    /// version of [`RegionBaseline::id`]`(content_hash)`.
    ///
    /// Like every lookup it starts no load and never waits: the caller first asks with
    /// `catalog.request_newest(RegionBaseline::id(content_hash), priority)`. `Pending` and
    /// `Failed` are the catalogue's own answers for that asset. `Ready` is also checked:
    /// if the asset published under that ID is of another kind, or is a baseline with
    /// another content hash (a cooker that named it wrongly), the answer is `Failed` with
    /// `ErrorKind::InvalidData`, the asset's ID and the published version.
    pub fn lookup(catalog: &dyn AssetCatalog, content_hash: u64) -> Readiness<Arc<Baseline>> {
        todo!("look the content-addressed ID up and check what it holds")
    }
}

impl Asset for RegionBaseline {
    const KIND: AssetKind = AssetKind::from_name("region_baseline");

    /// The format of `Baseline::encode`. It moves when `lactose-bundle`'s document format
    /// version does, in the same change.
    const FORMAT_VERSION: u32 = 1;
}

/// The loader of [`RegionBaseline`]: `Baseline::decode` over the cooked bytes, a pure
/// decoder that lives in a boundary crate.
///
/// Every [`Store`](crate::Store) registers it itself. It is public for a test that needs
/// baselines from another store (`lactose_asset_api::conformance::ReferenceStore`).
#[derive(Clone, Copy, Default, Debug)]
pub struct RegionBaselineLoader;

impl Loader for RegionBaselineLoader {
    type Asset = RegionBaseline;

    /// Fails as `Baseline::decode` fails (`InvalidData` for anything but the canonical
    /// encoding of a baseline, `VersionMismatch` for another document format,
    /// `ResourceExhausted` above `lactose_bundle::MAX_DOCUMENT_BYTES`), and with
    /// `ErrorKind::InvalidData` if `dependencies` is not empty.
    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<RegionBaseline> {
        todo!("decode the baseline and refuse declared dependencies")
    }
}
