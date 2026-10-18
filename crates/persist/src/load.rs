//! The load side (architecture sections 15.2 and 15.4): from a region's records to the
//! `RegionBundle` the input sync point loads, migrated to the registry's schemas and
//! validated first, so a refused region changes nothing.

use std::sync::Arc;

use lactose_asset_api::PinnedTable;
use lactose_bundle::Baseline;
use lactose_core::Result;
use lactose_ecs::RegionMode;
use lactose_sim::RegionBundle;

use crate::diff::RegionRecords;
use crate::migrate::Migrations;
use crate::schema::SchemaTable;

/// What a load needs beyond the records: the loading build's saved schemas and its
/// migrations. One per registry, shared by every region's load.
#[derive(Clone, Copy, Debug)]
pub struct Loader<'a> {
    schemas: &'a SchemaTable,
    migrations: &'a Migrations,
}

impl<'a> Loader<'a> {
    /// A loader over `schemas` and `migrations`.
    #[must_use]
    pub fn new(schemas: &'a SchemaTable, migrations: &'a Migrations) -> Self {
        Self { schemas, migrations }
    }

    /// The saved schemas this loader brings documents to.
    #[must_use]
    pub fn schemas(&self) -> &'a SchemaTable {
        self.schemas
    }

    /// The migrations this loader runs.
    #[must_use]
    pub fn migrations(&self) -> &'a Migrations {
        self.migrations
    }

    /// The same records at the registry's schemas: the overlay through
    /// [`Migrations::migrate_overlay`] and each resource through [`Migrations::migrate`]
    /// under the schema the save carried for it (the container keeps one per resource
    /// type beside the records). Records whose every schema matches come back equal, the
    /// overlay as the same `Arc`. Fails as those fail, and with `NotFound` for a resource
    /// type the table does not know or `InvalidData` for one that is not a saved resource.
    pub fn migrate(&self, records: &RegionRecords) -> Result<RegionRecords> {
        todo!("migrate the overlay and the resources, or return the records unchanged")
    }

    /// The bundle that loads `records` over `baseline` in `mode` with `assets`: what goes
    /// in `TickInput::loaded` for the region, for a region load, a one-frame or two-frame
    /// reload and a fresh process alike.
    ///
    /// In order: `baseline.content_hash()` must equal `records.baseline_hash`
    /// (`VersionMismatch` otherwise: the baseline was patched, so
    /// [`RegionRecords::rebase`] first); the records are [migrated](Self::migrate); the
    /// overlay, if any, passes `Overlay::check_against(baseline)`; every resource and every
    /// component value a record carries is validated against the registry
    /// (`TypeInfo::validate`, `InvalidData`), a transient component's bytes by presence
    /// alone, since the loader zeroes them; a baseline column whose schema is not the
    /// table's is `VersionMismatch` here rather than at the sync point (a baseline is
    /// never migrated: the cooker of this build re-cooks it, and the records are rebased
    /// onto the result). Then the bundle: `seed`, `mode`, `baseline`, the migrated overlay,
    /// the migrated resources, the transient keys and `assets`. Nothing is loaded here;
    /// the sync point validates again before `create_region`.
    ///
    /// `mode` is honoured under `RegionControl::Directed` (a client's `Replicated`, a
    /// test's choice) and ignored under `Anchored`, where the policy sets it; `assets` is
    /// the table `app` resolved and pinned for the region, or `None` if it has not, in
    /// which case the region is loaded `Frozen` until a table arrives through
    /// `TickInput::assets`.
    pub fn bundle(
        &self,
        records: &RegionRecords,
        baseline: Arc<Baseline>,
        mode: RegionMode,
        assets: Option<PinnedTable>,
    ) -> Result<RegionBundle> {
        todo!("check the hash, migrate, validate, then assemble the bundle")
    }
}
