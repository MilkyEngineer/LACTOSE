//! The overlay of a region: typed records against its cooked baseline (architecture
//! section 15.2).

use std::collections::BTreeSet;

use lactose_core::{ComponentKey, Error, ErrorKind, FieldId, NetId, Result};

use crate::codec::{Reader, Writer, invalid, strictly_ascending};
use crate::schema::{self, ComponentSchema};
use crate::{Baseline, ComponentData};

const MAGIC: [u8; 4] = *b"LOVL";

const TAG_SPAWN: u8 = 1;
const TAG_SPAWN_FROM_BASELINE: u8 = 2;
const TAG_DESTROY: u8 = 3;
const TAG_ADD_COMPONENT: u8 = 4;
const TAG_REMOVE_COMPONENT: u8 = 5;
const TAG_SET_FIELD: u8 = 6;
const TAG_OWNERSHIP_CHANGE: u8 = 7;

/// One overlay record. Every record is keyed by the [`NetId`] of the entity it describes.
///
/// Within one entity, records apply in the order the variants are declared.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Record {
    /// A runtime-spawned entity that resides in this region, with all its persistent
    /// components. `id` is a runtime ID.
    Spawn {
        /// The entity.
        id: NetId,
        /// Its components, in ascending key order once inside an [`Overlay`].
        components: Vec<ComponentData>,
    },
    /// A cooked entity of another region's baseline that now resides in this region, with
    /// all its persistent components at their current values, so that this region loads
    /// without the other. `id` is a cooked ID that is not a row of this region's baseline.
    SpawnFromBaseline {
        /// The entity.
        id: NetId,
        /// Its components, in ascending key order once inside an [`Overlay`].
        components: Vec<ComponentData>,
    },
    /// A cooked entity of this region's baseline that no longer resides here, because it
    /// was destroyed or moved to another region. `id` is a cooked ID, and no other record
    /// has it.
    Destroy {
        /// The entity.
        id: NetId,
    },
    /// A whole component written over the baseline row: one the row has gained, or, if the
    /// row already has the component, a replacement of all of it.
    AddComponent {
        /// The entity, a row of this region's baseline.
        id: NetId,
        /// The whole component.
        data: ComponentData,
    },
    /// A component the baseline row has lost.
    RemoveComponent {
        /// The entity, a row of this region's baseline.
        id: NetId,
        /// The component removed.
        component: ComponentKey,
    },
    /// One field of a baseline row's component whose value differs from the baseline.
    SetField {
        /// The entity, a row of this region's baseline.
        id: NetId,
        /// The component the field belongs to.
        component: ComponentKey,
        /// The field, by its explicit ID. The overlay's schema of `component` locates it.
        field: FieldId,
        /// The field's value, exactly as long as the schema says the field is.
        bytes: Vec<u8>,
    },
    /// A change of owning entity.
    OwnershipChange {
        /// The entity.
        id: NetId,
        /// The new owner, or `None` if the entity is no longer owned.
        owner: Option<NetId>,
    },
}

impl Record {
    /// The entity this record describes.
    #[must_use]
    pub fn id(&self) -> NetId {
        match *self {
            Self::Spawn { id, .. }
            | Self::SpawnFromBaseline { id, .. }
            | Self::Destroy { id }
            | Self::AddComponent { id, .. }
            | Self::RemoveComponent { id, .. }
            | Self::SetField { id, .. }
            | Self::OwnershipChange { id, .. } => id,
        }
    }

    fn tag(&self) -> u8 {
        match self {
            Self::Spawn { .. } => TAG_SPAWN,
            Self::SpawnFromBaseline { .. } => TAG_SPAWN_FROM_BASELINE,
            Self::Destroy { .. } => TAG_DESTROY,
            Self::AddComponent { .. } => TAG_ADD_COMPONENT,
            Self::RemoveComponent { .. } => TAG_REMOVE_COMPONENT,
            Self::SetField { .. } => TAG_SET_FIELD,
            Self::OwnershipChange { .. } => TAG_OWNERSHIP_CHANGE,
        }
    }

    /// The record order of the canonical form. No two records of an overlay share a key.
    fn key(&self) -> (NetId, u8, ComponentKey, FieldId) {
        let (component, field) = match *self {
            Self::AddComponent { ref data, .. } => (data.component, FieldId(0)),
            Self::RemoveComponent { component, .. } => (component, FieldId(0)),
            Self::SetField { component, field, .. } => (component, field),
            _ => (ComponentKey(0), FieldId(0)),
        };
        (self.id(), self.tag(), component, field)
    }

    fn spawns(&self) -> bool {
        matches!(self, Self::Spawn { .. } | Self::SpawnFromBaseline { .. })
    }

    /// The whole components this record carries.
    fn components(&self) -> &[ComponentData] {
        match self {
            Self::Spawn { components, .. } | Self::SpawnFromBaseline { components, .. } => {
                components
            }
            Self::AddComponent { data, .. } => core::slice::from_ref(data),
            _ => &[],
        }
    }

    /// Every component this record carries bytes of, whole or one field.
    fn data_components(&self) -> impl Iterator<Item = ComponentKey> + '_ {
        let field = match *self {
            Self::SetField { component, .. } => Some(component),
            _ => None,
        };
        self.components().iter().map(|data| data.component).chain(field)
    }
}

/// The overlay of one region: every difference between the region's current persistent
/// state and its cooked baseline, with the schema of every component it carries bytes of.
///
/// The same records are a region's part of a save, what unloading the region writes, and
/// what the server sends a client when the region becomes relevant. A document is immutable
/// and canonical: a record set has exactly one in-memory form and one encoding, whatever
/// order it was built in, and a state of the region has exactly one record set.
///
/// # The one way to say each thing
///
/// | Entity | Records |
/// |---|---|
/// | Runtime-spawned, residing here | `Spawn` with every persistent component; optionally `OwnershipChange`. Nothing else: a change to it is a change to its `Spawn` |
/// | Cooked in another region, residing here | `SpawnFromBaseline` with every persistent component; optionally `OwnershipChange`. Nothing else |
/// | Row of this region's baseline, changed | `AddComponent`, `RemoveComponent`, `SetField`, `OwnershipChange`. Never `SpawnFromBaseline` of its own row. A component is written whole by `AddComponent` or field by field by `SetField`, never both |
/// | Row of this region's baseline, destroyed or moved away | `Destroy`, alone |
/// | Runtime-spawned, destroyed or moved away | no record |
///
/// A cooked entity that moves from region A to region B is therefore `Destroy` in A's
/// overlay and `SpawnFromBaseline` in B's, so B loads without A. A runtime entity that
/// moves leaves A's overlay and becomes a `Spawn` in B's.
///
/// [`Overlay::new`] and [`Overlay::decode`] enforce everything in this table that can be
/// judged without the baseline. [`Overlay::check_against`] judges the rest. When the
/// baseline is patched, [`Overlay::rebase`] carries the overlay over.
///
/// # Encoding
///
/// After the shared header (see the crate documentation), in order:
///
/// | Part | Layout |
/// |---|---|
/// | baseline hash | `u64`, the content hash of the baseline the records were made against |
/// | schema table | as in [`Baseline`]; exactly the components some record carries bytes of |
/// | records | `u32` count, then each record, strictly ascending by (ID, tag, component key, field ID) |
/// | record | `u8` tag, `u64` [`NetId`], then the payload below |
///
/// | Tag | Record | Payload |
/// |---|---|---|
/// | 1 | `Spawn` | `u32` component count, then per component `u64` key and its bytes; strictly ascending by key |
/// | 2 | `SpawnFromBaseline` | as `Spawn` |
/// | 3 | `Destroy` | none |
/// | 4 | `AddComponent` | `u64` key and the component's bytes |
/// | 5 | `RemoveComponent` | `u64` key |
/// | 6 | `SetField` | `u64` key, `u16` field ID and the field's bytes |
/// | 7 | `OwnershipChange` | `u64` owner [`NetId`], zero for none |
///
/// Component and field bytes carry no length: the schema table gives it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Overlay {
    baseline_hash: u64,
    schemas: Vec<ComponentSchema>,
    records: Vec<Record>,
    hash: u64,
}

impl Overlay {
    /// Builds an overlay against the baseline whose content hash is `baseline_hash`, from
    /// records given in any order.
    ///
    /// `schemas` may hold more than the records use, for instance a whole registry; the
    /// overlay keeps those of the components it carries bytes of.
    ///
    /// Fails with `ErrorKind::InvalidData` if:
    ///
    /// - two records have the same entity, kind, component and field;
    /// - a `Spawn` has a cooked ID, or a `SpawnFromBaseline` or `Destroy` a runtime ID;
    /// - a runtime entity has records but no `Spawn`;
    /// - a `Destroy` shares its entity with any other record;
    /// - a spawned entity has an `AddComponent`, `RemoveComponent` or `SetField`;
    /// - an entity removes a component that it also adds or sets a field of, or sets a
    ///   field of a component it adds;
    /// - an entity is its own owner, is owned by an entity this overlay destroys, or the
    ///   ownership records form a cycle;
    /// - a spawn names a component twice;
    /// - a component has no schema or two, its bytes are not the size its schema states,
    ///   or a `SetField` names a field its schema lacks or has bytes of another size.
    ///
    /// Fails with `ErrorKind::ResourceExhausted` if the encoding would exceed
    /// [`MAX_DOCUMENT_BYTES`](crate::MAX_DOCUMENT_BYTES).
    pub fn new(
        baseline_hash: u64,
        schemas: impl IntoIterator<Item = ComponentSchema>,
        records: impl IntoIterator<Item = Record>,
    ) -> Result<Self> {
        let mut records: Vec<_> = records.into_iter().collect();
        for record in &mut records {
            if let Record::Spawn { components, .. } | Record::SpawnFromBaseline { components, .. } =
                record
            {
                components.sort_by_key(|data| data.component);
            }
        }
        records.sort_by_key(Record::key);
        let used = records.iter().flat_map(Record::data_components).collect();
        let schemas = schema::select(schemas, &used)?;
        Self::assemble(baseline_hash, schemas, records, None)
    }

    /// The content hash of the baseline these records were made against. If it differs
    /// from the hash of the baseline at hand, that baseline was patched: see
    /// [`Overlay::rebase`].
    #[must_use]
    pub fn baseline_hash(&self) -> u64 {
        self.baseline_hash
    }

    /// The schema of every component some record carries bytes of, strictly ascending by
    /// component.
    #[must_use]
    pub fn schemas(&self) -> &[ComponentSchema] {
        &self.schemas
    }

    /// The schema the overlay's bytes of `component` were written with.
    #[must_use]
    pub fn schema(&self, component: ComponentKey) -> Option<&ComponentSchema> {
        schema::find(&self.schemas, component)
    }

    /// The records, ordered by entity and, within an entity, in application order.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// The overlay digest a client compares against its cached copy (architecture section
    /// 9.3): the hash that ends the canonical encoding. It is kept from construction or
    /// decoding, so this is free.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.hash
    }

    /// Checks this overlay against the baseline it is about to be applied to: the part of
    /// the canonical form that cannot be judged without one.
    ///
    /// Fails with `ErrorKind::VersionMismatch` if `baseline` is not the baseline the
    /// overlay was made against (rebase it first), and with `ErrorKind::InvalidData` if:
    ///
    /// - a `SpawnFromBaseline` names a row of `baseline`;
    /// - any other record with a cooked ID names an entity that is not a row of
    ///   `baseline`;
    /// - a `RemoveComponent` or `SetField` names a component its row does not have.
    ///
    /// What is left for whoever applies the overlay, because it depends on state this
    /// crate does not see: that each owner exists, that ownership has no cycle once the
    /// baseline's own ownership is counted, that the overlay's schemas agree with the
    /// registry or are migrated, and every cross-region reference.
    pub fn check_against(&self, baseline: &Baseline) -> Result<()> {
        if self.baseline_hash != baseline.content_hash() {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                "overlay was made against another baseline",
            ));
        }
        for record in &self.records {
            let id = record.id();
            if !id.is_cooked() {
                continue;
            }
            let row = baseline.archetype_of(id);
            let sound = match *record {
                Record::SpawnFromBaseline { .. } => row.is_none(),
                Record::RemoveComponent { component, .. } | Record::SetField { component, .. } => {
                    row.is_some_and(|row| row.has(component))
                }
                Record::Spawn { .. }
                | Record::Destroy { .. }
                | Record::AddComponent { .. }
                | Record::OwnershipChange { .. } => row.is_some() || self.spawns(id),
            };
            if !sound {
                return Err(invalid("a record does not fit the baseline"));
            }
        }
        Ok(())
    }

    /// Carries this overlay over to a patched baseline (architecture section 15.2).
    ///
    /// Returns the overlay to use with `patched`, which passes
    /// [`check_against`](Self::check_against) it, and the records that were dropped, in
    /// order, so the caller can re-home those of an entity the patch moved to another
    /// region.
    ///
    /// - Every runtime-spawned entity is kept, with its records.
    /// - A cooked entity residing here from another region (`SpawnFromBaseline`) is kept.
    ///   If the patch made it a row of this baseline, its record is rewritten in the
    ///   canonical form for a local row, with the same meaning: an `AddComponent` for
    ///   each component it carries and a `RemoveComponent` for each component the row has
    ///   and it does not. Nothing is dropped for it.
    /// - Every record of any other cooked entity that is not a row of `patched` is
    ///   dropped, `Destroy` included.
    /// - For a row of `patched`: a `RemoveComponent` of a component the row does not have
    ///   is dropped, and so is a `SetField` of such a component. An `AddComponent` is
    ///   kept even if the row now has the component; it overwrites it whole.
    ///
    /// Whether a field still exists in the patched component is a schema question: the
    /// overlay keeps its own schema of the component, and migration decides.
    ///
    /// Fails only with `ErrorKind::ResourceExhausted`, if rewriting records pushed the
    /// encoding past [`MAX_DOCUMENT_BYTES`](crate::MAX_DOCUMENT_BYTES).
    pub fn rebase(&self, patched: &Baseline) -> Result<(Self, Vec<Record>)> {
        let mut kept = Vec::with_capacity(self.records.len());
        let mut dropped = Vec::new();
        for entity in self.records.chunk_by(|a, b| a.id() == b.id()) {
            let id = entity[0].id();
            let row = patched.archetype_of(id);
            match (row, entity.iter().any(Record::spawns)) {
                _ if !id.is_cooked() => kept.extend_from_slice(entity),
                (None, true) => kept.extend_from_slice(entity),
                (None, false) => dropped.extend_from_slice(entity),
                (Some(row), true) => {
                    for record in entity {
                        let Record::SpawnFromBaseline { components, .. } = record else {
                            kept.push(record.clone());
                            continue;
                        };
                        let carried = |component| {
                            components.binary_search_by_key(&component, |data| data.component)
                        };
                        let lost =
                            row.columns().filter(|column| carried(column.component).is_err());
                        kept.extend(lost.map(|column| Record::RemoveComponent {
                            id,
                            component: column.component,
                        }));
                        let written = components.iter().cloned();
                        kept.extend(written.map(|data| Record::AddComponent { id, data }));
                    }
                }
                (Some(row), false) => {
                    for record in entity {
                        let orphaned = match *record {
                            Record::RemoveComponent { component, .. }
                            | Record::SetField { component, .. } => !row.has(component),
                            _ => false,
                        };
                        if orphaned {
                            dropped.push(record.clone())
                        } else {
                            kept.push(record.clone())
                        }
                    }
                }
            }
        }
        kept.sort_by_key(Record::key);
        let used = kept.iter().flat_map(Record::data_components).collect();
        let schemas = schema::select(self.schemas.iter().cloned(), &used)?;
        let rebased = Self::assemble(patched.content_hash(), schemas, kept, None)?;
        Ok((rebased, dropped))
    }

    /// The canonical encoding. Equal overlays encode to equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let sealed = seal(self.baseline_hash, &self.schemas, &self.records);
        sealed.expect("the size was checked when the overlay was built").0
    }

    /// Decodes and validates untrusted bytes, such as an overlay received from the server.
    ///
    /// Fails with `ErrorKind::ResourceExhausted` for more than
    /// [`MAX_DOCUMENT_BYTES`](crate::MAX_DOCUMENT_BYTES), with `ErrorKind::VersionMismatch`
    /// for another format version, and with `ErrorKind::InvalidData` for anything that is
    /// not the canonical encoding of an overlay. A successful decode re-encodes to exactly
    /// `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut reader, hash) = Reader::open(bytes, MAGIC)?;
        let baseline_hash = reader.u64()?;

        let schemas = schema::read_table(&mut reader)?;
        // Lookups below are binary searches, so the order has to hold first.
        strictly_ascending(
            schemas.iter().map(ComponentSchema::component),
            "the schema table is not strictly ascending",
        )?;
        let schema_of = |component| {
            schema::find(&schemas, component)
                .ok_or(invalid("a record carries a component outside the schema table"))
        };
        let read_data = |reader: &mut Reader| -> Result<ComponentData> {
            let component = reader.component()?;
            let len = schema_of(component)?.element_size() as usize;
            Ok(ComponentData { component, bytes: reader.take(len)?.to_vec() })
        };

        // A record is at least its tag and ID.
        let count = reader.count(9)?;
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            let tag = reader.u8()?;
            let id = reader.net_id()?;
            records.push(match tag {
                TAG_SPAWN | TAG_SPAWN_FROM_BASELINE => {
                    // A component is at least its key.
                    let count = reader.count(8)?;
                    let components =
                        (0..count).map(|_| read_data(&mut reader)).collect::<Result<_>>()?;
                    if tag == TAG_SPAWN {
                        Record::Spawn { id, components }
                    } else {
                        Record::SpawnFromBaseline { id, components }
                    }
                }
                TAG_DESTROY => Record::Destroy { id },
                TAG_ADD_COMPONENT => Record::AddComponent { id, data: read_data(&mut reader)? },
                TAG_REMOVE_COMPONENT => {
                    Record::RemoveComponent { id, component: reader.component()? }
                }
                TAG_SET_FIELD => {
                    let component = reader.component()?;
                    let field = FieldId(reader.u16()?);
                    let Some(layout) = schema_of(component)?.field(field) else {
                        return Err(invalid("SetField names a field its schema lacks"));
                    };
                    let bytes = reader.take(layout.size as usize)?.to_vec();
                    Record::SetField { id, component, field, bytes }
                }
                TAG_OWNERSHIP_CHANGE => {
                    Record::OwnershipChange { id, owner: NetId::from_bits(reader.u64()?) }
                }
                _ => return Err(invalid("unknown record tag")),
            });
        }
        reader.finish()?;
        Self::assemble(baseline_hash, schemas, records, Some(hash))
    }

    /// Whether this overlay spawns the entity `id`.
    fn spawns(&self, id: NetId) -> bool {
        self.entity(id).iter().any(Record::spawns)
    }

    /// The records of the entity `id`, which are contiguous.
    fn entity(&self, id: NetId) -> &[Record] {
        entity_of(&self.records, id)
    }

    /// Checks every invariant the canonical form promises and completes the document.
    /// `hash` is the trailing hash when the parts were decoded from sealed bytes.
    fn assemble(
        baseline_hash: u64,
        schemas: Vec<ComponentSchema>,
        records: Vec<Record>,
        hash: Option<u64>,
    ) -> Result<Self> {
        strictly_ascending(
            records.iter().map(Record::key),
            "records are not strictly ascending, or two describe the same thing",
        )?;
        let used: BTreeSet<_> = records.iter().flat_map(Record::data_components).collect();
        schema::check_table(&schemas, &used)?;
        for record in &records {
            check_record(record, &schemas)?;
        }
        for entity in records.chunk_by(|a, b| a.id() == b.id()) {
            check_entity(entity)?;
        }
        check_ownership(&records)?;

        let hash = match hash {
            Some(hash) => hash,
            None => seal(baseline_hash, &schemas, &records)?.1,
        };
        Ok(Self { baseline_hash, schemas, records, hash })
    }
}

/// The records of the entity `id` within records sorted by entity.
fn entity_of(records: &[Record], id: NetId) -> &[Record] {
    let start = records.partition_point(|record| record.id() < id);
    let len = records[start..].partition_point(|record| record.id() == id);
    &records[start..start + len]
}

/// The checks on one record alone: its ID kind and the size of everything it carries.
fn check_record(record: &Record, schemas: &[ComponentSchema]) -> Result<()> {
    let schema_of = |component| schema::find(schemas, component).expect("the table was checked");
    for data in record.components() {
        if data.bytes.len() != schema_of(data.component).element_size() as usize {
            return Err(invalid("a component is not the size its schema states"));
        }
    }
    match record {
        Record::Spawn { id, components } => {
            if id.is_cooked() {
                return Err(invalid("Spawn names a cooked entity"));
            }
            spawn_components_ascend(components)
        }
        Record::SpawnFromBaseline { id, components } => {
            if !id.is_cooked() {
                return Err(invalid("SpawnFromBaseline names a runtime entity"));
            }
            spawn_components_ascend(components)
        }
        Record::Destroy { id } if !id.is_cooked() => Err(invalid("Destroy names a runtime entity")),
        Record::SetField { component, field, bytes, .. } => {
            match schema_of(*component).field(*field) {
                Some(layout) if layout.size as usize == bytes.len() => Ok(()),
                Some(_) => Err(invalid("a field value is not the size its schema states")),
                None => Err(invalid("SetField names a field its schema lacks")),
            }
        }
        Record::OwnershipChange { id, owner } if *owner == Some(*id) => {
            Err(invalid("an entity is its own owner"))
        }
        _ => Ok(()),
    }
}

/// The checks on the records of one entity together.
fn check_entity(entity: &[Record]) -> Result<()> {
    let destroyed = entity.iter().any(|record| matches!(record, Record::Destroy { .. }));
    if destroyed && entity.len() > 1 {
        return Err(invalid("a destroyed entity has other records"));
    }
    let spawned = entity.iter().any(Record::spawns);
    if !entity[0].id().is_cooked() && !spawned {
        return Err(invalid("a runtime entity has records but no Spawn"));
    }
    if spawned {
        let whole =
            |record: &Record| record.spawns() || matches!(record, Record::OwnershipChange { .. });
        if !entity.iter().all(whole) {
            return Err(invalid("a spawned entity has component or field records"));
        }
        return Ok(());
    }
    // Records of one kind are ordered by component, so each list is ascending and a
    // lookup is a binary search. That keeps an entity with many records linear-logarithmic.
    let of_kind = |select: fn(&Record) -> Option<ComponentKey>| -> Vec<ComponentKey> {
        entity.iter().filter_map(select).collect()
    };
    let added = of_kind(|record| match record {
        Record::AddComponent { data, .. } => Some(data.component),
        _ => None,
    });
    let removed = of_kind(|record| match *record {
        Record::RemoveComponent { component, .. } => Some(component),
        _ => None,
    });
    let within = |list: &[ComponentKey], component| list.binary_search(&component).is_ok();
    for record in entity {
        match *record {
            Record::AddComponent { ref data, .. } if within(&removed, data.component) => {
                return Err(invalid("an entity both adds and removes a component"));
            }
            Record::SetField { component, .. } if within(&removed, component) => {
                return Err(invalid("an entity sets a field of a component it removes"));
            }
            Record::SetField { component, .. } if within(&added, component) => {
                return Err(invalid("an entity sets a field of a component it adds whole"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The ownership checks that need no baseline: no owner this overlay destroys, and no
/// cycle made of ownership records alone.
fn check_ownership(records: &[Record]) -> Result<()> {
    // Ascending by entity, because the records are.
    let owned: Vec<(NetId, NetId)> = records
        .iter()
        .filter_map(|record| match *record {
            Record::OwnershipChange { id, owner: Some(owner) } => Some((id, owner)),
            _ => None,
        })
        .collect();
    for &(_, owner) in &owned {
        if matches!(entity_of(records, owner), [Record::Destroy { .. }]) {
            return Err(invalid("an entity is owned by one this overlay destroys"));
        }
    }
    // Follow each chain of owners. A chain that returns to an entity on itself is a cycle;
    // one that reaches an entity already cleared, or leaves the overlay, is not.
    const UNSEEN: u8 = 0;
    const ON_CHAIN: u8 = 1;
    const CLEARED: u8 = 2;
    let mut state = vec![UNSEEN; owned.len()];
    let mut chain = Vec::new();
    for start in 0..owned.len() {
        let mut at = start;
        while state[at] == UNSEEN {
            state[at] = ON_CHAIN;
            chain.push(at);
            match owned.binary_search_by_key(&owned[at].1, |&(id, _)| id) {
                Ok(next) if state[next] == ON_CHAIN => {
                    return Err(invalid("ownership records form a cycle"));
                }
                Ok(next) => at = next,
                Err(_) => break,
            }
        }
        for seen in chain.drain(..) {
            state[seen] = CLEARED;
        }
    }
    Ok(())
}

fn spawn_components_ascend(components: &[ComponentData]) -> Result<()> {
    strictly_ascending(
        components.iter().map(|data| data.component),
        "a spawn's components are not strictly ascending",
    )
}

fn seal(
    baseline_hash: u64,
    schemas: &[ComponentSchema],
    records: &[Record],
) -> Result<(Vec<u8>, u64)> {
    let mut writer = Writer::new(MAGIC);
    writer.u64(baseline_hash);
    schema::write_table(&mut writer, schemas);
    writer.count(records.len());
    let write_data = |writer: &mut Writer, data: &ComponentData| {
        writer.u64(data.component.0);
        writer.bytes(&data.bytes);
    };
    for record in records {
        writer.u8(record.tag());
        writer.u64(record.id().to_bits());
        match record {
            Record::Spawn { components, .. } | Record::SpawnFromBaseline { components, .. } => {
                writer.count(components.len());
                for data in components {
                    write_data(&mut writer, data);
                }
            }
            Record::Destroy { .. } => {}
            Record::AddComponent { data, .. } => write_data(&mut writer, data),
            Record::RemoveComponent { component, .. } => writer.u64(component.0),
            Record::SetField { component, field, bytes, .. } => {
                writer.u64(component.0);
                writer.u16(field.0);
                writer.bytes(bytes);
            }
            Record::OwnershipChange { owner, .. } => {
                writer.u64(owner.map_or(0, NetId::to_bits));
            }
        }
    }
    writer.finish()
}
