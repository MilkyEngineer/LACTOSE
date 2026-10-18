//! Region relevance (architecture sections 9.3 and 15.2): the message the server sends
//! when a region becomes relevant to a client, built from the region's
//! [`RegionExport`] through `lactose-persist`'s diff by the server's `app`, and what the
//! client builds a [`RegionBundle`] from without diffing; and the other changes a client
//! learns about a region.
//!
//! Boundary code may use this module: documents, keys and tables; no float, no clock.

use std::sync::Arc;

use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{ComponentKey, Error, ErrorKind, Result};
use lactose_ecs::{RegionId, RegionMode};
use lactose_sim::{RegionBundle, RegionExport};

/// The region's state beyond its cooked baseline, as the message carries it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OverlayPayload {
    /// The region is in its cooked state: no overlay.
    Cooked,
    /// The overlay itself.
    Full(Arc<Overlay>),
    /// The overlay's digest (`Overlay::digest`): the client holds a cached copy with that
    /// digest, from an earlier relevance or a save, and applies it.
    Digest(u64),
}

/// A region becoming relevant to a client: its overlay or a digest, with the seed and the
/// persistent region resources beside it (architecture section 9.3), the hash of the
/// baseline the overlay was made against, and the region's asset table if resolved. The
/// overlay is a region-scoped document, distinct from the row replication stream: the
/// region's entities then arrive through [`StagedFrame::spawns`](crate::StagedFrame::spawns)
/// and its rows through the stream.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionRelevance {
    /// The region.
    pub region: RegionId,
    /// The region seed, as exported.
    pub seed: u64,
    /// `Baseline::content_hash` of the cooked baseline the overlay was made against; the
    /// client fetches the baseline by it from its store.
    pub baseline_hash: u64,
    /// The overlay, or a digest, or nothing.
    pub overlay: OverlayPayload,
    /// The persistent region resources, as exported.
    pub resources: Vec<ComponentData>,
    /// The keys of the transient region resources, as exported, restored zeroed.
    pub transient_resources: Vec<ComponentKey>,
    /// The region's exact-version asset table, if resolved when the message was built. A
    /// table that arrives later comes as [`RegionChange::Assets`].
    pub assets: Option<VersionTable>,
}

impl RegionRelevance {
    /// The message for `export`, whose rows the server's `app` has diffed into `overlay`
    /// (`None` for a region still in its cooked state) against the baseline whose content
    /// hash is `baseline_hash`.
    #[must_use]
    pub fn from_export(
        export: &RegionExport,
        baseline_hash: u64,
        overlay: Option<Arc<Overlay>>,
        assets: Option<VersionTable>,
    ) -> Self {
        Self {
            region: export.region,
            seed: export.rows.seed,
            baseline_hash,
            overlay: overlay.map_or(OverlayPayload::Cooked, OverlayPayload::Full),
            resources: export.rows.resources.clone(),
            transient_resources: export.rows.transient_resources.clone(),
            assets,
        }
    }

    /// The same message with a `Full` overlay replaced by its digest, for a client known
    /// to hold a copy.
    #[must_use]
    pub fn with_digest(&self) -> Self {
        let overlay = match &self.overlay {
            OverlayPayload::Full(overlay) => OverlayPayload::Digest(overlay.digest()),
            other => other.clone(),
        };
        Self { overlay, ..self.clone() }
    }

    /// The bundle the client puts in `TickInput::loaded`: `baseline`, fetched by
    /// [`baseline_hash`](Self::baseline_hash); the overlay, or for a `Digest` the client's
    /// `cached` copy; the resources and transient keys; `assets` pinned by the client's
    /// `app` from the table; and `RegionMode::Replicated`. Fails with `VersionMismatch` if
    /// `baseline`'s content hash is not `baseline_hash`, if a `Digest` has no cached copy
    /// or the copy's digest differs, or if the overlay, carried or cached, was made against
    /// another baseline (`Overlay::check_against`).
    pub fn to_bundle(
        &self,
        baseline: Arc<Baseline>,
        cached: Option<&Arc<Overlay>>,
        assets: Option<PinnedTable>,
    ) -> Result<RegionBundle> {
        if baseline.content_hash() != self.baseline_hash {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "region {}: the baseline's content hash {:#018x} is not the message's \
                     {:#018x}",
                    self.region.0,
                    baseline.content_hash(),
                    self.baseline_hash
                ),
            ));
        }
        let overlay = match &self.overlay {
            OverlayPayload::Cooked => None,
            OverlayPayload::Full(overlay) => Some(Arc::clone(overlay)),
            OverlayPayload::Digest(digest) => match cached {
                Some(cached) if cached.digest() == *digest => Some(Arc::clone(cached)),
                Some(cached) => {
                    return Err(Error::new(
                        ErrorKind::VersionMismatch,
                        format!(
                            "region {}: the cached overlay's digest {:#018x} is not the \
                             message's {digest:#018x}",
                            self.region.0,
                            cached.digest()
                        ),
                    ));
                }
                None => {
                    return Err(Error::new(
                        ErrorKind::VersionMismatch,
                        format!(
                            "region {}: the message names overlay {digest:#018x} by digest and \
                             no copy is cached",
                            self.region.0
                        ),
                    ));
                }
            },
        };
        // A cached copy is checked as a carried one is: a digest names bytes, not the
        // baseline they were made against.
        if let Some(overlay) = &overlay {
            overlay.check_against(&baseline)?;
        }
        Ok(RegionBundle {
            seed: self.seed,
            mode: RegionMode::Replicated,
            baseline,
            overlay,
            resources: self.resources.clone(),
            transient_resources: self.transient_resources.clone(),
            assets,
        })
    }
}

/// What a client learns about one region in one frame, keyed by the region in
/// [`StagedFrame::regions`](crate::StagedFrame::regions). `app` applies it to the
/// `TickInput` of the tick: `Relevant` becomes a `loaded` bundle, `Irrelevant` an
/// `unloaded` entry, `Assets` an `assets` entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RegionChange {
    /// The region became relevant.
    Relevant(RegionRelevance),
    /// The region is no longer relevant: the client unloads it, which despawns its
    /// entities; the server sends a [`TombstoneCause::Left`](crate::TombstoneCause::Left)
    /// tombstone beside it for each of the client's entities residing in it, which
    /// releases their handles and slots.
    Irrelevant,
    /// The region's asset table was published or replaced (a hot reload, architecture
    /// section 9.3): the client pins it and puts it in `TickInput::assets`.
    Assets(VersionTable),
}
