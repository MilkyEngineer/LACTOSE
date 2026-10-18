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

    /// The same records at the registry's schemas, in canonical form: the overlay through
    /// [`Migrations::migrate_overlay`] with the records' `layouts`, and each resource
    /// through [`Migrations::migrate`] under the schema and digest the records carry for
    /// it; `resource_schemas` and `layouts` then state the registry's. Records whose every
    /// carried type is at the registry's version and digest come back equal, the overlay
    /// as the same `Arc`.
    ///
    /// A type whose persistence changed since the save takes one rule for a resource and a
    /// component alike: now transient, the saved value is dropped and the type is restored
    /// zeroed (a resource leaves `resources` and joins `transient_resources`; a component's
    /// bytes are never read, `migrate_overlay`); now persistent, the type is restored
    /// zeroed as a persistent value (a transient key leaves `transient_resources` and
    /// joins `resources` as the all-zero value under the registry's schema and digest; a
    /// component's zeroed bytes go through `migrate`'s persistent rule), which
    /// [`bundle`](Self::bundle) then validates like any value, so a type whose all-zero
    /// value is invalid refuses the region with `InvalidData`; now derived, the type is
    /// dropped (a transient key or a resource leaves the records; a component leaves its
    /// record), since a derived value is rebuilt on load and never comes from a document,
    /// and the build declared it so. Fails as the two migrations fail, with `NotFound` for
    /// a resource or transient type the table does not know, and with `InvalidData` for
    /// records that fail [`RegionRecords::check`] or for a resource whose type is a
    /// component.
    pub fn migrate(&self, records: &RegionRecords) -> Result<RegionRecords> {
        todo!("migrate the overlay and the resources, or return the records unchanged")
    }

    /// The bundle that loads `records` over `baseline` in `mode` with `assets`: what goes
    /// in `TickInput::loaded` for the region, for a region load, a one-frame or two-frame
    /// reload and a fresh process alike.
    ///
    /// In order: `records` passes [`check`](RegionRecords::check); `baseline.content_hash()`
    /// equals `records.baseline_hash` (`VersionMismatch` otherwise: the baseline was
    /// patched, so [`RegionRecords::rebase`] first); the records are [migrated](Self::migrate);
    /// the overlay, if any, passes `Overlay::check_against(baseline)`; every value a
    /// record or a resource carries is validated against the registry
    /// (`TypeInfo::validate`, `InvalidData`), a resource restored zeroed by `migrate`
    /// included and a transient component by presence alone, since it is zeroed; a
    /// baseline column whose schema is not the table's is
    /// `VersionMismatch` here rather than at the sync point (a baseline is never migrated:
    /// the cooker of this build re-cooks it, and the records are rebased onto the result;
    /// a baseline carries no layout digest, so a same-size layout change in a cooked column
    /// is not seen here until `lactose-bundle` carries the digest, D129). Then the bundle:
    /// `seed`, `mode`, `baseline`, the migrated overlay, the migrated resources, the
    /// transient keys and `assets`. Nothing is loaded here; the sync point validates again
    /// before `create_region`.
    ///
    /// `mode` is honoured under `RegionControl::Directed` (a client's `Replicated`, a
    /// test's choice) and ignored under `Anchored`, where the policy sets it; `assets` is
    /// the table `app` resolved and pinned for the region, or `None` if it has not, in
    /// which case the region is loaded `Frozen` until a table arrives through
    /// `TickInput::assets`. Neither is in a save (`README.md`, "Policy").
    pub fn bundle(
        &self,
        records: &RegionRecords,
        baseline: Arc<Baseline>,
        mode: RegionMode,
        assets: Option<PinnedTable>,
    ) -> Result<RegionBundle> {
        todo!("check, compare the hash, migrate, validate, then assemble the bundle")
    }
}
