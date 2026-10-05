//! The overlay of a region: typed records against its cooked baseline (architecture
//! section 15.2).

use std::collections::BTreeMap;

use lactose_core::{ComponentKey, FieldId, NetId, Result};

use crate::codec::{Reader, Writer, check_component_len, invalid, strictly_ascending};
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
    /// without the other. `id` is a cooked ID. If this region's own baseline has a row
    /// with the same ID, this record replaces it.
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
    /// A component the entity has gained since the baseline.
    AddComponent {
        /// The entity.
        id: NetId,
        /// The whole component.
        data: ComponentData,
    },
    /// A component the entity has lost since the baseline.
    RemoveComponent {
        /// The entity.
        id: NetId,
        /// The component removed.
        component: ComponentKey,
    },
    /// One field whose value differs from the baseline.
    SetField {
        /// The entity.
        id: NetId,
        /// The component the field belongs to.
        component: ComponentKey,
        /// The schema version of that component when the value was written.
        schema_version: u32,
        /// The field, by its explicit ID.
        field: FieldId,
        /// The field's value.
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

    /// Every (component, schema version, data length) this record carries.
    fn data(&self) -> impl Iterator<Item = (ComponentKey, u32, usize)> + '_ {
        let (many, one) = match self {
            Self::Spawn { components, .. } | Self::SpawnFromBaseline { components, .. } => {
                (components.as_slice(), None)
            }
            Self::AddComponent { data, .. } => (core::slice::from_ref(data), None),
            Self::SetField { component, schema_version, bytes, .. } => {
                (&[][..], Some((*component, *schema_version, bytes.len())))
            }
            _ => (&[][..], None),
        };
        many.iter().map(|data| (data.component, data.schema_version, data.bytes.len())).chain(one)
    }
}

/// The overlay of one region: every difference between the region's current persistent
/// state and its cooked baseline.
///
/// The same records are a region's part of a save, what unloading the region writes, and
/// what the server sends a client when the region becomes relevant. A document is immutable
/// and always canonical, so the same record set has exactly one in-memory form and one
/// encoding, whatever order it was built in.
///
/// # Rules
///
/// - An entity that only changed has `AddComponent`, `RemoveComponent`, `SetField` and
///   `OwnershipChange` records against its baseline row.
/// - A cooked entity that moves from region A to region B becomes `Destroy` in A's overlay
///   and `SpawnFromBaseline`, carrying every persistent component, in B's. A runtime
///   entity that moves leaves A's overlay and becomes a `Spawn` in B's.
/// - When the baseline is patched, [`Overlay::rebase`] carries the overlay over.
///
/// # Encoding
///
/// After the shared header (see the crate documentation), in order:
///
/// | Part | Layout |
/// |---|---|
/// | baseline hash | `u64`, the content hash of the baseline the records were made against |
/// | schema table | `u32` count, then per component `u64` key and `u32` schema version, strictly ascending by key; exactly the components some record carries data for |
/// | records | `u32` count, then each record, strictly ascending by (ID, tag, component key, field ID) |
/// | record | `u8` tag, `u64` [`NetId`], then the payload below |
///
/// | Tag | Record | Payload |
/// |---|---|---|
/// | 1 | `Spawn` | `u32` component count, then per component `u64` key, `u32` length, data; strictly ascending by key |
/// | 2 | `SpawnFromBaseline` | as `Spawn` |
/// | 3 | `Destroy` | none |
/// | 4 | `AddComponent` | `u64` key, `u32` length, data |
/// | 5 | `RemoveComponent` | `u64` key |
/// | 6 | `SetField` | `u64` key, `u16` field ID, `u32` length, data |
/// | 7 | `OwnershipChange` | `u64` owner [`NetId`], zero for none |
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Overlay {
    baseline_hash: u64,
    records: Vec<Record>,
}

impl Overlay {
    /// Builds an overlay against the baseline whose content hash is `baseline_hash`, from
    /// records given in any order.
    ///
    /// Fails with `ErrorKind::InvalidData` if:
    ///
    /// - two records have the same entity, kind, component and field;
    /// - a `Spawn` has a cooked ID, or a `SpawnFromBaseline` or `Destroy` a runtime ID;
    /// - a `Destroy` shares its entity with any other record;
    /// - an entity removes a component that it also adds or sets a field of;
    /// - an entity is made its own owner;
    /// - a spawn names a component twice;
    /// - component or field data is larger than
    ///   [`MAX_COMPONENT_BYTES`](crate::MAX_COMPONENT_BYTES);
    /// - a component has two schema versions.
    pub fn new(baseline_hash: u64, records: impl IntoIterator<Item = Record>) -> Result<Self> {
        let mut records: Vec<_> = records.into_iter().collect();
        for record in &mut records {
            if let Record::Spawn { components, .. } | Record::SpawnFromBaseline { components, .. } =
                record
            {
                components.sort_by_key(|data| data.component);
            }
        }
        records.sort_by_key(Record::key);
        let overlay = Self { baseline_hash, records };
        overlay.validate()?;
        Ok(overlay)
    }

    /// The content hash of the baseline these records were made against. If it differs
    /// from the hash of the baseline at hand, that baseline was patched: see
    /// [`Overlay::rebase`].
    #[must_use]
    pub fn baseline_hash(&self) -> u64 {
        self.baseline_hash
    }

    /// The records, ordered by entity and, within an entity, in application order.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Carries this overlay over to a patched baseline (architecture section 15.2).
    ///
    /// Returns the overlay to use with `patched`, and the records that were dropped. Every
    /// runtime-spawned entity is kept, and so is every record except a `SetField` whose
    /// target no longer exists: one whose entity is neither spawned by this overlay nor a
    /// row of `patched`, or whose component is neither on that row nor added by this
    /// overlay. Whether the field still exists in the component is a schema question and
    /// is decided by migration, not here.
    ///
    /// Records of the other kinds that name an entity `patched` no longer has are kept, as
    /// the architecture states; whoever applies an overlay skips them.
    #[must_use]
    pub fn rebase(&self, patched: &Baseline) -> (Self, Vec<Record>) {
        let mut kept = Vec::with_capacity(self.records.len());
        let mut dropped = Vec::new();
        for entity in self.records.chunk_by(|a, b| a.id() == b.id()) {
            let spawned = entity.iter().any(Record::spawns);
            let row = patched.archetype_of(entity[0].id());
            let added = components_of(entity, |record| match record {
                Record::AddComponent { data, .. } => Some(data.component),
                _ => None,
            });
            let adds = |component| added.binary_search(&component).is_ok();
            for record in entity {
                let orphaned = match *record {
                    Record::SetField { component, .. } if !spawned => {
                        !row.is_some_and(|row| row.column(component).is_some() || adds(component))
                    }
                    _ => false,
                };
                if orphaned { dropped.push(record.clone()) } else { kept.push(record.clone()) }
            }
        }
        (Self { baseline_hash: patched.content_hash(), records: kept }, dropped)
    }

    /// The canonical encoding. Equal overlays encode to equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.seal().0
    }

    /// The overlay digest a client compares against its cached copy (architecture section
    /// 9.3): the hash that ends the canonical encoding. Computing it encodes the document.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.seal().1
    }

    fn seal(&self) -> (Vec<u8>, u64) {
        let mut writer = Writer::new(MAGIC);
        writer.u64(self.baseline_hash);
        let schemas = self.schemas().expect("an overlay is validated when it is built");
        writer.len(schemas.len());
        for (component, schema_version) in schemas {
            writer.u64(component.0);
            writer.u32(schema_version);
        }
        writer.len(self.records.len());
        let write_data = |writer: &mut Writer, data: &ComponentData| {
            writer.u64(data.component.0);
            writer.len(data.bytes.len());
            writer.bytes(&data.bytes);
        };
        for record in &self.records {
            writer.u8(record.tag());
            writer.u64(record.id().to_bits());
            match record {
                Record::Spawn { components, .. } | Record::SpawnFromBaseline { components, .. } => {
                    writer.len(components.len());
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
                    writer.len(bytes.len());
                    writer.bytes(bytes);
                }
                Record::OwnershipChange { owner, .. } => {
                    writer.u64(owner.map_or(0, NetId::to_bits));
                }
            }
        }
        writer.finish()
    }

    /// Decodes and validates untrusted bytes, such as an overlay received from the server.
    ///
    /// Fails with `ErrorKind::VersionMismatch` for another format version and with
    /// `ErrorKind::InvalidData` for anything that is not the canonical encoding of an
    /// overlay. A successful decode re-encodes to exactly `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::open(bytes, MAGIC)?;
        let baseline_hash = reader.u64()?;

        let mut schemas = BTreeMap::new();
        for _ in 0..reader.count(12)? {
            let (component, schema_version) = (reader.component()?, reader.u32()?);
            if schemas.last_key_value().is_some_and(|(&last, _)| component <= last) {
                return Err(invalid("schema table is not strictly ascending"));
            }
            schemas.insert(component, schema_version);
        }
        let read_data = |reader: &mut Reader| -> Result<ComponentData> {
            let component = reader.component()?;
            let Some(&schema_version) = schemas.get(&component) else {
                return Err(invalid("record carries a component outside the schema table"));
            };
            Ok(ComponentData {
                component,
                schema_version,
                bytes: reader.component_bytes()?.to_vec(),
            })
        };

        let mut records = Vec::new();
        // A record is at least its tag and ID.
        for _ in 0..reader.count(9)? {
            let tag = reader.u8()?;
            let id = reader.net_id()?;
            records.push(match tag {
                TAG_SPAWN | TAG_SPAWN_FROM_BASELINE => {
                    // A component is at least its key and length.
                    let count = reader.count(12)?;
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
                    let Some(&schema_version) = schemas.get(&component) else {
                        return Err(invalid("record carries a component outside the schema table"));
                    };
                    let field = FieldId(reader.u16()?);
                    let bytes = reader.component_bytes()?.to_vec();
                    Record::SetField { id, component, schema_version, field, bytes }
                }
                TAG_OWNERSHIP_CHANGE => {
                    Record::OwnershipChange { id, owner: NetId::from_bits(reader.u64()?) }
                }
                _ => return Err(invalid("unknown record tag")),
            });
        }
        reader.finish()?;

        let overlay = Self { baseline_hash, records };
        overlay.validate()?;
        if overlay.schemas()? != schemas {
            return Err(invalid("schema table lists a component no record carries"));
        }
        Ok(overlay)
    }

    /// The schema version of every component some record carries data for, which must
    /// agree across records.
    fn schemas(&self) -> Result<BTreeMap<ComponentKey, u32>> {
        let mut schemas = BTreeMap::new();
        for (component, schema_version, _) in self.records.iter().flat_map(Record::data) {
            if *schemas.entry(component).or_insert(schema_version) != schema_version {
                return Err(invalid("a component has two schema versions in one overlay"));
            }
        }
        Ok(schemas)
    }

    /// Checks every invariant the canonical form promises.
    fn validate(&self) -> Result<()> {
        strictly_ascending(
            self.records.iter().map(Record::key),
            "records are not strictly ascending, or two describe the same thing",
        )?;
        for record in &self.records {
            for (_, _, len) in record.data() {
                check_component_len(len)?;
            }
            match record {
                Record::Spawn { id, components } => {
                    if id.is_cooked() {
                        return Err(invalid("Spawn names a cooked entity"));
                    }
                    spawn_components_ascend(components)?;
                }
                Record::SpawnFromBaseline { id, components } => {
                    if !id.is_cooked() {
                        return Err(invalid("SpawnFromBaseline names a runtime entity"));
                    }
                    spawn_components_ascend(components)?;
                }
                Record::Destroy { id } if !id.is_cooked() => {
                    return Err(invalid("Destroy names a runtime entity"));
                }
                Record::OwnershipChange { id, owner } if *owner == Some(*id) => {
                    return Err(invalid("an entity is its own owner"));
                }
                _ => {}
            }
        }
        for entity in self.records.chunk_by(|a, b| a.id() == b.id()) {
            let destroyed = entity.iter().any(|record| matches!(record, Record::Destroy { .. }));
            if destroyed && entity.len() > 1 {
                return Err(invalid("a destroyed entity has other records"));
            }
            let removed = components_of(entity, |record| match *record {
                Record::RemoveComponent { component, .. } => Some(component),
                _ => None,
            });
            let removes = |component| removed.binary_search(&component).is_ok();
            for record in entity {
                let contradicted = match *record {
                    Record::AddComponent { ref data, .. } => removes(data.component),
                    Record::SetField { component, .. } => removes(component),
                    _ => false,
                };
                if contradicted {
                    return Err(invalid("an entity both removes and writes a component"));
                }
            }
        }
        self.schemas().map(drop)
    }
}

/// The components `select` picks from one entity's records, ascending because records of
/// one kind are ordered by component. A lookup is then a binary search, which keeps
/// validating an entity with many records linear-logarithmic in an untrusted document.
fn components_of(
    entity: &[Record],
    select: impl Fn(&Record) -> Option<ComponentKey>,
) -> Vec<ComponentKey> {
    entity.iter().filter_map(select).collect()
}

fn spawn_components_ascend(components: &[ComponentData]) -> Result<()> {
    strictly_ascending(
        components.iter().map(|data| data.component),
        "a spawn's components are not strictly ascending",
    )
}
