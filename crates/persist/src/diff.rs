//! The world overlay (architecture section 15.2): a region's export diffed against its
//! cooked baseline into the typed records of `lactose_bundle`, and the per-region unit a
//! save and the relevance message are built from.

use std::sync::Arc;

use lactose_bundle::{Baseline, ComponentData, ComponentSchema, Overlay, Record};
use lactose_core::{ComponentKey, Result};
use lactose_ecs::{RegionId, RegionRows};
use lactose_sim::RegionExport;

use crate::schema::SchemaTable;

/// The overlay of `rows`, a region's state as `World::export_region` copied it out,
/// against `baseline`, the cooked baseline the region was loaded from: every difference
/// between the two as the records of architecture section 15.2, in one canonical
/// `Overlay` made against `baseline.content_hash()`.
///
/// This is the one diff. A save calls it per region through
/// [`RegionRecords::from_export`]; the server's `app` calls it on the export of a region
/// that became relevant to a client and hands the result to
/// `lactose_net_api::RegionRelevance::from_export` (`None` when `records()` is empty, which
/// the message sends as `Cooked`). The rule per entity and the canonical form are stated in
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
/// beside it the region's seed, its persistent region resources and the keys of its
/// transient ones, since `Overlay` has no field for them (`docs/verification.md`).
///
/// Equal state gives equal records: every field is canonical, the overlay by construction
/// and the resources by this crate's canonicalisation, and the resources are in ascending
/// key order. Two `RegionRecords` compare by content, the overlay included.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionRecords {
    /// The region.
    pub region: RegionId,
    /// The region seed, as exported; the first element of every RNG key drawn in the
    /// region, so a reload draws the streams the original did.
    pub seed: u64,
    /// `Baseline::content_hash` of the cooked baseline the overlay was made against. A
    /// load fetches the baseline by it. Present also when `overlay` is `None`, since a
    /// region in its cooked state still names the baseline it was loaded from.
    pub baseline_hash: u64,
    /// The overlay, or `None` for a region in its cooked state: the diff produced no
    /// record. Shared, as `RegionBundle::overlay` and the relevance message share it.
    pub overlay: Option<Arc<Overlay>>,
    /// The persistent region resources, ascending by key, each canonical.
    pub resources: Vec<ComponentData>,
    /// The schema each resource in `resources` was written with, strictly ascending by
    /// component and exactly one per resource (`InvalidData` otherwise, wherever the
    /// records are built or decoded): what lets a resource of an older schema version be
    /// migrated, as an overlay's own schema table does for its records.
    pub resource_schemas: Vec<ComponentSchema>,
    /// The keys of the transient region resources, ascending: present and hashed, never
    /// saved by value, restored zeroed.
    pub transient_resources: Vec<ComponentKey>,
}

impl RegionRecords {
    /// The records of `export`, a region as an unload or a save copied it out, against
    /// `baseline`: [`diff`] over `export.rows`, `None` when it is empty, the seed and the
    /// transient keys copied, each resource canonicalised and its schema taken from the
    /// table. Fails as [`diff`] fails, and with `NotFound` or `InvalidData` for a resource
    /// the table does not know, that is not a saved resource, or whose bytes are not its
    /// schema's size.
    ///
    /// Independent per region: `app` runs one call per region of a `SavePoint` on
    /// background workers and inserts each result into a [`Container`](crate::Container).
    pub fn from_export(
        export: &RegionExport,
        baseline: &Baseline,
        schemas: &SchemaTable,
    ) -> Result<Self> {
        todo!("diff the rows, canonicalise the resources, keep the seed and transient keys")
    }

    /// Carries these records over to `patched`, a re-cooked baseline whose content hash
    /// differs from [`baseline_hash`](Self::baseline_hash): `Overlay::rebase`, after which
    /// `baseline_hash` names `patched` and the overlay fits it. Returns the records the
    /// rebase dropped, in order, so the caller can re-home those of a cooked entity the
    /// patch moved to another region (the cross-region pass is not in this crate yet).
    /// With no overlay only the hash changes. Fails only as `Overlay::rebase` fails.
    pub fn rebase(&mut self, patched: &Baseline) -> Result<Vec<Record>> {
        todo!("rebase the overlay and take the patched baseline's hash")
    }
}
