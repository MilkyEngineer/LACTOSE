//! The world overlay (architecture section 15.2): a region's export diffed against its
//! cooked baseline into the typed records of `lactose_bundle`, and the per-region unit a
//! save and the relevance message are built from.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_asset_api::VersionTable;
use lactose_bundle::{Archetype, Baseline, ComponentData, ComponentSchema, Overlay, Record};
use lactose_core::{ComponentKey, Error, ErrorKind, NetId, Result, StableKey};
use lactose_ecs::{ExportedRow, OwnedBy, RegionId, RegionRows};
use lactose_net_api::{OverlayPayload, RegionRelevance};
use lactose_reflect::{Persistence, Reflect, TypeInfo, TypeRole};
use lactose_sim::RegionExport;

use crate::codec::invalid;
use crate::schema::{LayoutDigest, Saved, SchemaTable};

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
    for schema in baseline.schemas() {
        let info = component_info(schemas, schema.component())?;
        // A saved column is compared byte for byte, so it must be in the layout the
        // export's bytes are in; a derived one is never read.
        if let Some(saved) = schemas.saved(info.key())
            && *schema != saved.schema
        {
            return Err(invalid(format!(
                "the baseline's schema of `{}` is not the registry's",
                info.name()
            )));
        }
    }
    strictly_ascending(rows.rows.iter().map(|row| row.id), "the export's rows")?;

    // Every baseline row, ascending, with where it lies.
    let mut cooked: Vec<(NetId, &Archetype, usize)> = baseline
        .archetypes()
        .iter()
        .flat_map(|archetype| {
            archetype.ids().iter().enumerate().map(move |(row, &id)| (id, archetype, row))
        })
        .collect();
    cooked.sort_unstable_by_key(|&(id, ..)| id);

    let mut differ = Differ { schemas, records: Vec::new(), ours: Vec::new(), theirs: Vec::new() };
    let mut exported = rows.rows.iter().peekable();
    let mut cooked = cooked.into_iter().peekable();
    loop {
        match (exported.peek(), cooked.peek()) {
            (None, None) => break,
            (Some(row), Some(&(id, ..))) if row.id == id => {
                let (_, archetype, index) = cooked.next().expect("peeked");
                differ.both(exported.next().expect("peeked"), archetype, index)?;
            }
            (Some(row), Some(&(id, ..))) if row.id < id => {
                differ.spawn(exported.next().expect("peeked"))?;
            }
            (Some(_), None) => differ.spawn(exported.next().expect("peeked"))?,
            (_, Some(_)) => {
                let (id, ..) = cooked.next().expect("peeked");
                differ.records.push(Record::Destroy { id });
            }
        }
    }
    Overlay::new(baseline.content_hash(), schemas.schemas().cloned(), differ.records)
}

/// The records of one diff as they are found, and two scratch buffers for comparing a
/// value with its baseline without allocating per component.
struct Differ<'a> {
    schemas: &'a SchemaTable,
    records: Vec<Record>,
    ours: Vec<u8>,
    theirs: Vec<u8>,
}

impl Differ<'_> {
    /// An exported row the baseline lacks.
    fn spawn(&mut self, row: &ExportedRow) -> Result<()> {
        strictly_ascending(row.components.iter().map(|data| data.component), "a row's components")?;
        strictly_ascending(row.transient.iter().copied(), "a row's transient components")?;
        let mut components = Vec::with_capacity(row.components.len() + row.transient.len());
        for data in &row.components {
            let saved = exported_value(self.schemas, data)?;
            let mut bytes = data.bytes.clone();
            saved.canonicalise(&mut bytes);
            components.push(ComponentData { component: data.component, bytes });
        }
        for &component in &row.transient {
            components.push(zeroed(transient(self.schemas, component)?));
        }
        let id = row.id;
        self.records.push(if id.is_cooked() {
            Record::SpawnFromBaseline { id, components }
        } else {
            Record::Spawn { id, components }
        });
        if row.owner.is_some() {
            self.records.push(Record::OwnershipChange { id, owner: row.owner });
        }
        Ok(())
    }

    /// A row of both: row `index` of `archetype` in the baseline.
    fn both(&mut self, row: &ExportedRow, archetype: &Archetype, index: usize) -> Result<()> {
        let id = row.id;
        let mut owner = None;
        let mut columns = archetype.columns().peekable();
        let mut exported = row.components.iter().peekable();
        strictly_ascending(row.components.iter().map(|data| data.component), "a row's components")?;
        // The persistent components, merged by key with the baseline's persistent columns.
        loop {
            let column = loop {
                match columns.peek() {
                    Some(column) if column.component == OwnedBy::KEY => {
                        owner = baseline_owner(value_of(column, index))?;
                        columns.next();
                    }
                    Some(column) if !persistent(self.schemas, column.component)? => {
                        columns.next();
                    }
                    other => break other.copied(),
                }
            };
            match (exported.peek(), column) {
                (None, None) => break,
                (Some(data), Some(column)) if data.component == column.component => {
                    let saved = exported_value(self.schemas, data)?;
                    if self.changed(saved, &data.bytes, value_of(&column, index)) {
                        let bytes = self.ours.clone();
                        let data = ComponentData { component: data.component, bytes };
                        self.records.push(Record::AddComponent { id, data });
                    }
                    exported.next();
                    columns.next();
                }
                (Some(data), Some(column)) if data.component < column.component => {
                    self.add(id, data)?;
                    exported.next();
                }
                (Some(data), None) => {
                    self.add(id, data)?;
                    exported.next();
                }
                (_, Some(column)) => {
                    let component = column.component;
                    self.records.push(Record::RemoveComponent { id, component });
                    columns.next();
                }
            }
        }
        // The transient components, by presence alone.
        strictly_ascending(row.transient.iter().copied(), "a row's transient components")?;
        let mut had = BTreeSet::new();
        for column in archetype.columns() {
            let info = component_info(self.schemas, column.component)?;
            if info.persistence() == Persistence::Transient {
                had.insert(column.component);
            }
        }
        for &component in &row.transient {
            let saved = transient(self.schemas, component)?;
            if !had.remove(&component) {
                self.records.push(Record::AddComponent { id, data: zeroed(saved) });
            }
        }
        for component in had {
            self.records.push(Record::RemoveComponent { id, component });
        }
        if row.owner != owner {
            self.records.push(Record::OwnershipChange { id, owner: row.owner });
        }
        Ok(())
    }

    /// A persistent component the baseline row lacks.
    fn add(&mut self, id: NetId, data: &ComponentData) -> Result<()> {
        let saved = exported_value(self.schemas, data)?;
        let mut bytes = data.bytes.clone();
        saved.canonicalise(&mut bytes);
        self.records.push(Record::AddComponent {
            id,
            data: ComponentData { component: data.component, bytes },
        });
        Ok(())
    }

    /// Whether `exported` and `cooked`, two values of one saved type, differ once both are
    /// canonical. Leaves the canonical `exported` in `self.ours`.
    fn changed(&mut self, saved: &Saved, exported: &[u8], cooked: &[u8]) -> bool {
        self.ours.clear();
        self.ours.extend_from_slice(exported);
        saved.canonicalise(&mut self.ours);
        self.theirs.clear();
        self.theirs.extend_from_slice(cooked);
        saved.canonicalise(&mut self.theirs);
        self.ours != self.theirs
    }
}

/// Row `index`'s value in `column`.
fn value_of<'a>(column: &lactose_bundle::Column<'a>, index: usize) -> &'a [u8] {
    let size = column.element_size as usize;
    &column.bytes[index * size..(index + 1) * size]
}

/// The owner a baseline `OwnedBy` value names: `None` for the empty reference.
fn baseline_owner(bytes: &[u8]) -> Result<Option<NetId>> {
    let value: OwnedBy = bytemuck::checked::try_pod_read_unaligned(bytes)
        .map_err(|_| invalid("an invalid `OwnedBy` in the baseline".to_owned()))?;
    match value.owner.get() {
        None => Ok(None),
        Some(StableKey::Net(owner)) => Ok(Some(owner)),
        Some(other) => {
            Err(invalid(format!("a baseline `OwnedBy` names {other:?}, which is not a `Net` key")))
        }
    }
}

/// The registry's description of `component`, which must be a registered component.
fn component_info(schemas: &SchemaTable, component: ComponentKey) -> Result<&'static TypeInfo> {
    let info = schemas.info(component).ok_or_else(|| unknown(component))?;
    if info.role() != TypeRole::Component {
        return Err(invalid(format!("the resource `{}` is among a row's components", info.name())));
    }
    Ok(info)
}

/// Whether the baseline column `component` is persistent and so compared by value.
fn persistent(schemas: &SchemaTable, component: ComponentKey) -> Result<bool> {
    Ok(component_info(schemas, component)?.persistence() == Persistence::Persistent)
}

/// What the table keeps for `data`, a persistent component of an exported row, after the
/// refusals the diff states.
fn exported_value<'a>(schemas: &'a SchemaTable, data: &ComponentData) -> Result<&'a Saved> {
    let info = component_info(schemas, data.component)?;
    if info.key() == OwnedBy::KEY || info.persistence() != Persistence::Persistent {
        return Err(invalid(format!(
            "`{}` is among a row's persistent components but is not one",
            info.name()
        )));
    }
    let saved = schemas.saved(data.component).expect("a persistent type is saved");
    sized(info, saved, &data.bytes)?;
    Ok(saved)
}

/// What the table keeps for `component`, a transient component of an exported row.
fn transient(schemas: &SchemaTable, component: ComponentKey) -> Result<&Saved> {
    let info = component_info(schemas, component)?;
    if info.persistence() != Persistence::Transient {
        return Err(invalid(format!(
            "`{}` is among a row's transient components but is not one",
            info.name()
        )));
    }
    Ok(schemas.saved(component).expect("a transient type is saved"))
}

/// The all-zero value of a saved type: how a transient value is carried.
fn zeroed(saved: &Saved) -> ComponentData {
    let size = saved.schema.element_size() as usize;
    ComponentData { component: saved.schema.component(), bytes: vec![0; size] }
}

/// Fails unless `bytes` is the size `saved`'s schema states.
fn sized(info: &TypeInfo, saved: &Saved, bytes: &[u8]) -> Result<()> {
    let size = saved.schema.element_size() as usize;
    if bytes.len() == size {
        Ok(())
    } else {
        Err(invalid(format!("`{}` is {size} bytes, got {}", info.name(), bytes.len())))
    }
}

fn strictly_ascending<T: Ord>(items: impl IntoIterator<Item = T>, what: &str) -> Result<()> {
    let mut items = items.into_iter();
    let Some(mut previous) = items.next() else { return Ok(()) };
    for item in items {
        if item <= previous {
            return Err(invalid(format!("{what} are not strictly ascending")));
        }
        previous = item;
    }
    Ok(())
}

pub(crate) fn unknown(component: ComponentKey) -> Error {
    Error::new(ErrorKind::NotFound, format!("type {:#018x} is not registered", component.0))
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
        let overlay = diff(&export.rows, baseline, schemas)?;
        let overlay = (!overlay.records().is_empty()).then(|| Arc::new(overlay));
        let mut resources = Vec::with_capacity(export.rows.resources.len());
        for data in &export.rows.resources {
            let (info, saved) = resource(schemas, data.component, Persistence::Persistent)?;
            sized(info, saved, &data.bytes)?;
            let mut bytes = data.bytes.clone();
            saved.canonicalise(&mut bytes);
            resources.push(ComponentData { component: data.component, bytes });
        }
        resources.sort_by_key(|data| data.component);
        let mut transient_resources = export.rows.transient_resources.clone();
        for &component in &transient_resources {
            resource(schemas, component, Persistence::Transient)?;
        }
        transient_resources.sort_unstable();
        let resource_schemas = resources
            .iter()
            .map(|data| schemas.saved(data.component).expect("checked above").schema.clone())
            .collect();
        let carried = overlay.iter().flat_map(|overlay| overlay.schemas());
        let layouts = carried
            .map(ComponentSchema::component)
            .chain(resources.iter().map(|data| data.component))
            .map(|component| (component, schemas.layout(component).expect("a saved type")))
            .collect();
        let records = Self {
            region: export.region,
            seed: export.rows.seed,
            baseline_hash: baseline.content_hash(),
            overlay,
            resources,
            resource_schemas,
            transient_resources,
            layouts,
        };
        records.check()?;
        Ok(records)
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
        if let Some(overlay) = &self.overlay {
            if overlay.records().is_empty() {
                return Err(invalid("the records hold an empty overlay, not `None`".to_owned()));
            }
            if overlay.baseline_hash() != self.baseline_hash {
                return Err(invalid(format!(
                    "the records name baseline {:#018x} and their overlay {:#018x}",
                    self.baseline_hash,
                    overlay.baseline_hash()
                )));
            }
        }
        strictly_ascending(self.resources.iter().map(|data| data.component), "the resources")?;
        if self.resource_schemas.len() != self.resources.len() {
            return Err(invalid(format!(
                "{} resources have {} schemas",
                self.resources.len(),
                self.resource_schemas.len()
            )));
        }
        for (data, schema) in self.resources.iter().zip(&self.resource_schemas) {
            if schema.component() != data.component {
                return Err(invalid(format!(
                    "the schema of resource {:#018x} names {:#018x}",
                    data.component.0,
                    schema.component().0
                )));
            }
            if schema.element_size() as usize != data.bytes.len() {
                return Err(invalid(format!(
                    "resource {:#018x} is {} bytes and its schema states {}",
                    data.component.0,
                    data.bytes.len(),
                    schema.element_size()
                )));
            }
        }
        strictly_ascending(self.transient_resources.iter().copied(), "the transient resources")?;
        for &component in &self.transient_resources {
            if self.resources.binary_search_by_key(&component, |data| data.component).is_ok() {
                return Err(invalid(format!(
                    "resource {:#018x} is both saved and transient",
                    component.0
                )));
            }
        }
        let carried: BTreeSet<ComponentKey> = self
            .overlay
            .iter()
            .flat_map(|overlay| overlay.schemas())
            .map(ComponentSchema::component)
            .chain(self.resources.iter().map(|data| data.component))
            .collect();
        if let Some(missing) =
            carried.iter().find(|component| !self.layouts.contains_key(component))
        {
            return Err(invalid(format!("no layout digest for {:#018x}", missing.0)));
        }
        if let Some(extra) = self.layouts.keys().find(|component| !carried.contains(component)) {
            return Err(invalid(format!(
                "a layout digest for {:#018x}, which the records do not carry",
                extra.0
            )));
        }
        Ok(())
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
        let (overlay, dropped) = match &self.overlay {
            Some(overlay) => {
                let (rebased, dropped) = overlay.rebase(patched)?;
                ((!rebased.records().is_empty()).then(|| Arc::new(rebased)), dropped)
            }
            None => (None, Vec::new()),
        };
        self.baseline_hash = patched.content_hash();
        self.overlay = overlay;
        let carried: BTreeSet<ComponentKey> = self
            .overlay
            .iter()
            .flat_map(|overlay| overlay.schemas())
            .map(ComponentSchema::component)
            .chain(self.resources.iter().map(|data| data.component))
            .collect();
        self.layouts.retain(|component, _| carried.contains(component));
        Ok(dropped)
    }
}

/// The description and saved form of the region resource `component`, which must be a
/// registered resource of `persistence`.
fn resource(
    schemas: &SchemaTable,
    component: ComponentKey,
    persistence: Persistence,
) -> Result<(&'static TypeInfo, &Saved)> {
    let info = schemas.info(component).ok_or_else(|| unknown(component))?;
    if info.role() != TypeRole::Resource || info.persistence() != persistence {
        return Err(invalid(format!(
            "`{}` is exported as a {persistence:?} region resource but is not one",
            info.name()
        )));
    }
    Ok((info, schemas.saved(component).expect("a saved type")))
}
