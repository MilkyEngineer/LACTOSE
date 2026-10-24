//! The load side (architecture sections 15.2 and 15.4): from a region's records to the
//! `RegionBundle` the input sync point loads, migrated to the registry's schemas and
//! validated first, so a refused region changes nothing.

use std::collections::BTreeSet;
use std::sync::Arc;

use lactose_asset_api::PinnedTable;
use lactose_bundle::{Baseline, ComponentData, ComponentSchema, Record};
use lactose_core::{ComponentKey, Error, ErrorKind, NetId, Result};
use lactose_ecs::RegionMode;
use lactose_reflect::{Persistence, TypeInfo, TypeRole};
use lactose_sim::RegionBundle;

use crate::codec::invalid;
use crate::diff::{RegionRecords, unknown};
use crate::migrate::Migrations;
use crate::schema::{Saved, SchemaTable, document_schema};

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
    /// A type whose persistence changed since the save: now transient, the saved value is
    /// dropped and the type is restored zeroed (a resource leaves `resources` and joins
    /// `transient_resources`; a component's bytes are never read, `migrate_overlay`). Now
    /// persistent, a resource and a component part ways, since a document records which
    /// resources were transient and not which components were: a transient key leaves
    /// `transient_resources` and joins `resources` as the all-zero value under the
    /// registry's schema and digest, and no step runs (a key has no saved version for one
    /// to start from); a component is read by `migrate`'s persistent rule from the schema
    /// and digest it was saved under, like any component, so a type made persistent loads
    /// only if it kept the version and layout it was saved under, or has the steps from
    /// that version, which run over the all-zero old value. Either way
    /// [`bundle`](Self::bundle) validates the restored value like any other, so one that is
    /// invalid refuses the region with `InvalidData`. Now derived, the type is dropped (a
    /// transient key or a resource leaves the records; a component leaves its record,
    /// `migrate_overlay`) and an overlay left with no record becomes `None`, since a
    /// derived value is rebuilt on load and never comes from a document, and the build
    /// declared it so. Fails as the two migrations fail, with `NotFound` for a resource or
    /// transient type the table does not know, and with `InvalidData` for records that
    /// fail [`RegionRecords::check`] or for a resource whose type is a component.
    pub fn migrate(&self, records: &RegionRecords) -> Result<RegionRecords> {
        records.check()?;
        let schemas = self.schemas;
        let overlay = match &records.overlay {
            Some(overlay) => {
                let migrated =
                    self.migrations.migrate_overlay(overlay, &records.layouts, schemas)?;
                (!migrated.records().is_empty()).then_some(migrated)
            }
            None => None,
        };
        let mut resources = Vec::with_capacity(records.resources.len());
        let mut transient_resources = BTreeSet::new();
        for (data, schema) in records.resources.iter().zip(&records.resource_schemas) {
            let info = resource_of(schemas, data.component)?;
            match info.persistence() {
                Persistence::Derived => {}
                Persistence::Transient => {
                    transient_resources.insert(data.component);
                }
                Persistence::Persistent => {
                    let layout = records.layouts[&data.component];
                    let mut bytes = self.migrations.migrate(info, schema, layout, &data.bytes)?;
                    saved(schemas, data.component).canonicalise(&mut bytes);
                    resources.push(ComponentData { component: data.component, bytes });
                }
            }
        }
        for &component in &records.transient_resources {
            let info = resource_of(schemas, component)?;
            match info.persistence() {
                Persistence::Derived => {}
                Persistence::Transient => {
                    transient_resources.insert(component);
                }
                Persistence::Persistent => {
                    let size = saved(schemas, component).schema.element_size() as usize;
                    resources.push(ComponentData { component, bytes: vec![0; size] });
                }
            }
        }
        resources.sort_by_key(|data| data.component);
        let resource_schemas =
            resources.iter().map(|data| saved(schemas, data.component).schema.clone()).collect();
        let carried = overlay.iter().flat_map(|overlay| overlay.schemas());
        let layouts = carried
            .map(ComponentSchema::component)
            .chain(resources.iter().map(|data| data.component))
            .map(|component| (component, saved(schemas, component).layout))
            .collect();
        let migrated = RegionRecords {
            region: records.region,
            seed: records.seed,
            baseline_hash: records.baseline_hash,
            overlay,
            resources,
            resource_schemas,
            transient_resources: transient_resources.into_iter().collect(),
            layouts,
        };
        migrated.check()?;
        Ok(migrated)
    }

    /// The bundle that loads `records` over `baseline` in `mode` with `assets`: what goes
    /// in `TickInput::loaded` for the region, for a region load, a one-frame or two-frame
    /// reload and a fresh process alike.
    ///
    /// In order: `records` passes [`check`](RegionRecords::check); `baseline.content_hash()`
    /// equals `records.baseline_hash` (`VersionMismatch` otherwise: the baseline was
    /// patched, so [`migrate`](Self::migrate) and [`RegionRecords::rebase`] first); the
    /// records are [migrated](Self::migrate); the overlay, if any, passes
    /// `Overlay::check_against(baseline)`; every value a
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
        records.check()?;
        if baseline.content_hash() != records.baseline_hash {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "the records of region {} were made against baseline {:#018x}, not \
                     {:#018x}: rebase them first",
                    records.region.0,
                    records.baseline_hash,
                    baseline.content_hash()
                ),
            ));
        }
        let records = self.migrate(records)?;
        if let Some(overlay) = &records.overlay {
            overlay.check_against(&baseline)?;
            for record in overlay.records() {
                match record {
                    Record::Spawn { components, .. }
                    | Record::SpawnFromBaseline { components, .. } => {
                        for data in components {
                            self.validate(data)?;
                        }
                    }
                    Record::AddComponent { data, .. } => self.validate(data)?,
                    Record::SetField { id, component, field, bytes } => {
                        let info = self.schemas.info(*component).expect("migrated");
                        if info.persistence() != Persistence::Persistent {
                            continue;
                        }
                        let mut value = self.baseline_value(&baseline, *id, info)?;
                        info.write_field(&mut value, *field, bytes)?;
                        info.validate(&value)?;
                    }
                    Record::Destroy { .. }
                    | Record::RemoveComponent { .. }
                    | Record::OwnershipChange { .. } => {}
                }
            }
        }
        for data in &records.resources {
            self.validate(data)?;
        }
        for schema in baseline.schemas() {
            self.baseline_schema(schema)?;
        }
        Ok(RegionBundle {
            seed: records.seed,
            mode,
            baseline,
            overlay: records.overlay,
            resources: records.resources,
            transient_resources: records.transient_resources,
            assets,
        })
    }

    /// Validates `data`, a value the migrated records carry; a transient one counts by
    /// presence alone.
    fn validate(&self, data: &ComponentData) -> Result<()> {
        let info = self.schemas.info(data.component).expect("migrated records name known types");
        match info.persistence() {
            Persistence::Persistent => info.validate(&data.bytes),
            Persistence::Transient | Persistence::Derived => Ok(()),
        }
    }

    /// The value of `info`'s type in row `id` of `baseline`, which `check_against` has
    /// shown the row has.
    fn baseline_value(&self, baseline: &Baseline, id: NetId, info: &TypeInfo) -> Result<Vec<u8>> {
        let column = baseline.schema(info.key()).expect("the row has the component");
        self.baseline_schema(column)?;
        let archetype = baseline.archetype_of(id).expect("a cooked row");
        let index = archetype.ids().binary_search(&id).expect("a row of its archetype");
        let column = archetype.column(info.key()).expect("the row has the component");
        let size = column.element_size as usize;
        Ok(column.bytes[index * size..(index + 1) * size].to_vec())
    }

    /// Fails unless `schema`, a baseline's, is the one the registry derives for its type.
    fn baseline_schema(&self, schema: &ComponentSchema) -> Result<()> {
        let info =
            self.schemas.info(schema.component()).ok_or_else(|| unknown(schema.component()))?;
        if *schema != document_schema(info)? {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "the baseline's schema of `{}` is not the registry's: re-cook it",
                    info.name()
                ),
            ));
        }
        Ok(())
    }
}

/// The registry's description of the region resource `component`.
fn resource_of(schemas: &SchemaTable, component: ComponentKey) -> Result<&'static TypeInfo> {
    let info = schemas.info(component).ok_or_else(|| unknown(component))?;
    if info.role() != TypeRole::Resource {
        return Err(invalid(format!("the component `{}` is among the resources", info.name())));
    }
    Ok(info)
}

/// What the table keeps for `component`, a type `migrate` has shown is saved.
fn saved(schemas: &SchemaTable, component: ComponentKey) -> &Saved {
    schemas.saved(component).expect("a saved type")
}
