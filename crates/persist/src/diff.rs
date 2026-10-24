//! The world overlay (architecture section 15.2): a region's export diffed against its
//! cooked baseline into the typed records of `lactose_bundle`, and the per-region unit a
//! save and the relevance message are built from.

use std::collections::BTreeMap;
use std::sync::Arc;

use lactose_asset_api::VersionTable;
use lactose_bundle::{Baseline, ComponentData, ComponentSchema, Overlay, Record};
use lactose_core::{ComponentKey, Result};
use lactose_ecs::{RegionId, RegionRows};
use lactose_net_api::{OverlayPayload, RegionRelevance};
use lactose_sim::RegionExport;

use crate::schema::{LayoutDigest, SchemaTable};

/// The overlay of `rows`, a region's state as `World::export_region` copied it out,
/// against `baseline`, the cooked baseline the region was loaded from: every difference
/// between the two as the records of architecture section 15.2, in one canonical
/// `Overlay` made against `baseline.content_hash()`.
///
/// This is the one diff; [`RegionRecords::from_export`] is the one caller a save and the
/// relevance message share. The rule per entity and the canonical form are stated in
/// `README.md` ("The diff"); in short: a baseline row's component whose canonical bytes
/// differ is written whole as `AddComponent`, never as `SetField`; a component the row
/// gained is `AddComponent`, one it lost `RemoveComponent`; a changed owner is
/// `OwnershipChange`; a baseline row the export lacks is `Destroy`; a row the baseline
/// lacks is `Spawn` or `SpawnFromBaseline` with every persistent component; a transient
/// component is compared by presence alone and carried as zeroed bytes; a `derived` column
/// of the baseline is not compared.
///
/// Pure: the result is a function of the two arguments and the table, with no clock, no
/// float arithmetic and no order but key order, so the overlay's digest is the same on
/// every machine (D101: one record set per state).
///
/// Fails with `NotFound` for a component of either side the table does not know; with
/// `InvalidData` for a resource type among a row's components, a value whose bytes are not
/// the size the table's schema states, or a baseline `OwnedBy` whose reference is not a
/// `Net` key; and as `Overlay::new` fails.
pub fn diff(rows: &RegionRows, baseline: &Baseline, schemas: &SchemaTable) -> Result<Overlay> {
    todo!("compare each exported row with its baseline row and build the overlay")
}

/// One region's part of a save, and what the relevance message carries (architecture
/// sections 9.3 and 15.2): the overlay against the baseline named by its content hash, and
/// beside it the region's seed, its persistent region resources with their schemas, the
/// keys of its transient resources, and the layout digest of every type the records carry
/// bytes of, since `Overlay` has no field for any of them (`docs/verification.md`).
///
/// Equal state gives equal records: every field is canonical, the overlay by construction
/// and the resources by this crate's canonicalisation, and every list is in ascending key
/// order. The canonical form is [`check`](Self::check), and every function of this crate
/// that builds, takes or writes records holds it, so two `RegionRecords` that compare
/// unequal describe different states, except that records migrated from an older build
/// keep the saved bytes of a now-transient component, which no loader reads. Two
/// `RegionRecords` compare by content, the overlay included.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionRecords {
    /// The region.
    pub region: RegionId,
    /// The region seed, as exported; the first element of every RNG key drawn in the
    /// region, so a reload draws the streams the original did.
    pub seed: u64,
    /// `Baseline::content_hash` of the cooked baseline the overlay was made against. A
    /// load fetches the baseline by it. Present also when `overlay` is `None`, since a
    /// region in its cooked state still names the baseline it was loaded from; equal to
    /// `overlay.baseline_hash()` when there is one.
    pub baseline_hash: u64,
    /// The overlay, or `None` for a region in its cooked state: the diff produced no
    /// record. Never `Some` of an empty overlay. Shared, as `RegionBundle::overlay` and the
    /// relevance message share it.
    pub overlay: Option<Arc<Overlay>>,
    /// The persistent region resources, strictly ascending by key, each canonical.
    pub resources: Vec<ComponentData>,
    /// The schema each resource in `resources` was written with: one per resource, in the
    /// same order, each of its resource's size. What lets a resource of an older schema
    /// version be migrated, as an overlay's own schema table does for its records.
    pub resource_schemas: Vec<ComponentSchema>,
    /// The keys of the transient region resources, strictly ascending and disjoint from
    /// the types of `resources`: present and hashed, never saved by value, restored
    /// zeroed.
    pub transient_resources: Vec<ComponentKey>,
    /// The layout digest of every type the records carry bytes of, at the version they
    /// were written: exactly the components of the overlay's schema table and the types of
    /// `resources`, no more and no fewer. What tells a same-size layout change from the
    /// layout the bytes were written in ([`LayoutDigest`]); the relevance message does not
    /// carry it, since both ends share a build.
    pub layouts: BTreeMap<ComponentKey, LayoutDigest>,
}

impl RegionRecords {
    /// The records of `export`, a region as an unload or a save copied it out, against
    /// `baseline`: [`diff`] over `export.rows`, `None` when it is empty, the seed and the
    /// transient keys copied, each resource canonicalised with its schema and digest taken
    /// from the table, and `layouts` filled for every type carried. The result passes
    /// [`check`](Self::check). Fails as [`diff`] fails, and with `NotFound` or
    /// `InvalidData` for a resource the table does not know, that is not a saved resource,
    /// or whose bytes are not its schema's size.
    ///
    /// Independent per region: `app` runs one call per region of a `SavePoint` on
    /// background workers, each followed by
    /// [`encode_section`](Self::encode_section), and assembles the container afterwards.
    pub fn from_export(
        export: &RegionExport,
        baseline: &Baseline,
        schemas: &SchemaTable,
    ) -> Result<Self> {
        todo!("diff the rows, canonicalise the resources, keep the seed, keys and digests")
    }

    /// Whether these records are in canonical form: `overlay` is `None` or holds a record;
    /// `baseline_hash` equals the overlay's; `resources` is strictly ascending by key with
    /// `resource_schemas` one per resource in the same order, each schema naming its
    /// resource's type and stating its size; `transient_resources` is strictly ascending
    /// and names no type `resources` holds; and `layouts` has exactly one entry per
    /// component the overlay carries bytes of and per resource. Fails with `InvalidData`
    /// naming the first failure. Every builder, writer and decoder of records in this
    /// crate holds this, so a caller that assembles records by hand runs it before handing
    /// them over.
    pub fn check(&self) -> Result<()> {
        todo!("check each canonical rule in order")
    }

    /// The relevance message for these records (architecture section 9.3): the region,
    /// seed, baseline hash, resources and transient keys copied, the overlay as
    /// `OverlayPayload::Full` or `Cooked` when there is none, and `assets` as the server's
    /// `app` resolved the region's table, or `None` if it has not. The one rule for when a
    /// message says `Cooked` is therefore `from_export`'s, and the message's resources are
    /// the save's, canonical. Digests and resource schemas stay behind: both ends share a
    /// build by the handshake's protocol hash.
    #[must_use]
    pub fn to_relevance(&self, assets: Option<VersionTable>) -> RegionRelevance {
        RegionRelevance {
            region: self.region,
            seed: self.seed,
            baseline_hash: self.baseline_hash,
            overlay: match &self.overlay {
                Some(overlay) => OverlayPayload::Full(Arc::clone(overlay)),
                None => OverlayPayload::Cooked,
            },
            resources: self.resources.clone(),
            transient_resources: self.transient_resources.clone(),
            assets,
        }
    }

    /// Carries these records over to `patched`, a re-cooked baseline whose content hash
    /// differs from [`baseline_hash`](Self::baseline_hash): `Overlay::rebase`, after which
    /// `baseline_hash` names `patched` and the overlay fits it, or `None` if nothing is
    /// left. Returns the records the rebase dropped, in order, so the caller can re-home
    /// those of a cooked entity the patch moved to another region (the cross-region pass
    /// is not in this crate yet). With no overlay only the hash changes. `layouts` keeps
    /// the entries of the types still carried. Fails only as `Overlay::rebase` fails.
    pub fn rebase(&mut self, patched: &Baseline) -> Result<Vec<Record>> {
        todo!("rebase the overlay and take the patched baseline's hash")
    }
}
