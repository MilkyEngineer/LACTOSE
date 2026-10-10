//! Bulk copy of region rows to and from `lactose_bundle` documents (architecture sections
//! 15.2 and 15.4).

use std::collections::{BTreeMap, BTreeSet};

use lactose_bundle::{Baseline, ComponentData, ComponentSchema, Overlay, Record};
use lactose_core::{
    ComponentKey, EntityRef, Error, ErrorKind, NetId, PredictionKey, Result, StableKey,
};
use lactose_reflect::{Persistence, Reflect, TypeInfo, TypeRole};

use crate::command::{ErasedComponent, Payload};
use crate::region::{InRegion, OwnedBy, PredictedOrigin, RegionId};
use crate::storage::typed;
use crate::world::World;

/// Every persistent component of one entity, as [`World::export_region`](crate::World::export_region)
/// copies it out. `EntityRef` fields are written as stored, which is the target's origin
/// key: a reference to a predicted entity stays `Predicted` and resolves after a load
/// through the target's [`PredictedOrigin`](crate::PredictedOrigin), so the bytes, and the
/// hash over them, are the same before and after. Bytes are copied as laid out in memory
/// and not canonicalised; `lactose-persist` canonicalises scalars when it serialises through
/// `lactose-reflect` (architecture section 8.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExportedRow {
    /// The entity's `NetId`.
    pub id: NetId,
    /// The owning entity, if any. [`OwnedBy`](crate::OwnedBy) is carried here and is not
    /// among `components`.
    pub owner: Option<NetId>,
    /// The persistent components other than `ecs::owned_by`, in ascending key order, each
    /// exactly as the component is laid out in memory. [`PredictedOrigin`](crate::PredictedOrigin)
    /// is among them when the entity has one.
    pub components: Vec<ComponentData>,
    /// The transient components the entity has, in ascending key order: their values are
    /// rewritten every tick and not saved, but their presence is the entity's archetype,
    /// which the systems that run over it and the world hash depend on (architecture
    /// section 15.5). `lactose-persist` encodes presence as it sees fit, for instance as
    /// the component with zeroed bytes in the entity's records, ignores non-persistent
    /// components when it diffs against the baseline, and `load_region` stores any
    /// transient component a document carries as zeroed, whatever its bytes.
    pub transient: Vec<ComponentKey>,
}

/// The state of one region, as [`World::export_region`](crate::World::export_region)
/// copies it out: rows in ascending `NetId` order and resources in ascending key order, so
/// two exports of equal state are equal whatever the layout. `lactose-persist` diffs the
/// rows against the cooked baseline into an [`Overlay`](lactose_bundle::Overlay); the same
/// records serve a save, an unload and what the server sends a client when the region
/// becomes relevant.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RegionRows {
    /// The region seed.
    pub seed: u64,
    /// The region's reflected resources, persistent ones only.
    pub resources: Vec<ComponentData>,
    /// The rows.
    pub rows: Vec<ExportedRow>,
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// The schema a document must carry for `info`: its version, size and the layout of its
/// fields, padding excluded.
pub(crate) fn schema_of(info: &TypeInfo) -> ComponentSchema {
    let fields = info.layout().fields().iter().filter_map(|field| {
        Some(lactose_bundle::FieldLayout {
            id: field.id()?,
            offset: field.offset() as u32,
            size: field.size() as u32,
        })
    });
    ComponentSchema::new(info.key(), info.version(), info.layout().size() as u32, fields)
        .expect("a registered layout is a well-formed schema")
}

/// A row being assembled from the baseline and the overlay.
#[derive(Default)]
struct Row {
    components: BTreeMap<ComponentKey, Vec<u8>>,
    /// An `OwnershipChange` record's owner: `Some(None)` releases ownership.
    owner: Option<Option<NetId>>,
}

pub(crate) fn load_region(
    world: &mut World,
    region: RegionId,
    baseline: &Baseline,
    overlay: Option<&Overlay>,
) -> Result<()> {
    if !world.regions.contains_key(&region) {
        return Err(Error::new(ErrorKind::NotFound, "the region is not loaded"));
    }
    if world.entities_in(region).next().is_some() {
        return Err(invalid("the region is not empty"));
    }
    if let Some(overlay) = overlay {
        overlay.check_against(baseline)?;
    }

    let schemas = baseline.schemas().iter().chain(overlay.into_iter().flat_map(Overlay::schemas));
    for schema in schemas {
        let info = world.registry.get(schema.component()).ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!("component {:#018x} is not registered", schema.component().0),
            )
        })?;
        if info.role() != TypeRole::Component || info.key() == InRegion::KEY {
            return Err(invalid(format!("`{}` cannot be loaded as a row", info.name())));
        }
        if *schema != schema_of(info) {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!("the document's schema of `{}` is not the registry's", info.name()),
            ));
        }
    }

    let destroyed: BTreeSet<NetId> = overlay
        .into_iter()
        .flat_map(Overlay::records)
        .filter_map(|record| match record {
            Record::Destroy { id } => Some(*id),
            _ => None,
        })
        .collect();
    let mut rows: BTreeMap<NetId, Row> = BTreeMap::new();
    for archetype in baseline.archetypes() {
        for (index, &id) in archetype.ids().iter().enumerate() {
            if destroyed.contains(&id) {
                continue;
            }
            let row = rows.entry(id).or_default();
            for column in archetype.columns() {
                let size = column.element_size as usize;
                row.components.insert(
                    column.component,
                    column.bytes[index * size..(index + 1) * size].to_vec(),
                );
            }
        }
    }
    for record in overlay.into_iter().flat_map(Overlay::records) {
        match record {
            Record::Spawn { id, components } | Record::SpawnFromBaseline { id, components } => {
                let row = rows.entry(*id).or_default();
                for data in components {
                    row.components.insert(data.component, data.bytes.clone());
                }
            }
            Record::Destroy { .. } => {}
            Record::AddComponent { id, data } => {
                rows.entry(*id).or_default().components.insert(data.component, data.bytes.clone());
            }
            Record::RemoveComponent { id, component } => {
                rows.entry(*id).or_default().components.remove(component);
            }
            Record::SetField { id, component, field, bytes } => {
                let info = world.registry.get(*component).expect("checked above");
                let value = rows
                    .entry(*id)
                    .or_default()
                    .components
                    .get_mut(component)
                    .ok_or_else(|| invalid("a `SetField` names a component the row lacks"))?;
                info.write_field(value, *field, bytes)?;
            }
            Record::OwnershipChange { id, owner } => {
                rows.entry(*id).or_default().owner = Some(*owner)
            }
        }
    }

    // A transient component in a document stands for its presence: it loads zeroed, as
    // the input sync point leaves it. Then every value is validated before anything binds
    // or spawns, so a document with one bad row changes nothing.
    for row in rows.values_mut() {
        for (&component, bytes) in &mut row.components {
            let info = world.registry.get(component).expect("checked above");
            if info.persistence() == Persistence::Transient {
                bytes.fill(0);
            }
            info.validate(bytes)?;
        }
    }

    // Keys first, so an owner loaded in the same region resolves to its origin key.
    let mut keys = BTreeMap::new();
    let mut predicted_keys = BTreeSet::new();
    for (&id, row) in &rows {
        let key = match row.components.get(&PredictedOrigin::KEY) {
            Some(bytes) => {
                let origin: PredictedOrigin = bytemuck::checked::try_pod_read_unaligned(bytes)
                    .map_err(|_| invalid("an invalid `PredictedOrigin`"))?;
                let predicted = PredictionKey {
                    player: origin.player,
                    input_tick: origin.input_tick,
                    ordinal: origin.ordinal,
                };
                let bound = world.alias(predicted);
                let bound_back = world.aliases.by_net.get(&id);
                if bound.is_some_and(|net| net != id)
                    || bound_back.is_some_and(|&back| back != predicted)
                    || !predicted_keys.insert(predicted)
                {
                    return Err(invalid("a predicted origin is bound to another entity"));
                }
                StableKey::Predicted(predicted)
            }
            None => StableKey::Net(id),
        };
        if world.contains(key) || world.contains(StableKey::Net(id)) {
            return Err(invalid(format!("{key:?} is already live")));
        }
        keys.insert(id, key);
    }
    for (&id, &key) in &keys {
        if let StableKey::Predicted(predicted) = key {
            world.bind(predicted, id)?;
        }
    }

    let stamp = world.bump();
    for (id, mut row) in rows {
        match row.owner {
            Some(Some(owner)) => {
                let owner =
                    keys.get(&owner).copied().unwrap_or(world.canonical(StableKey::Net(owner)));
                let bytes = bytemuck::bytes_of(&OwnedBy { owner: EntityRef::new(owner) }).to_vec();
                row.components.insert(OwnedBy::KEY, bytes);
            }
            Some(None) => {
                row.components.remove(&OwnedBy::KEY);
            }
            None => {}
        }
        let components = row
            .components
            .into_iter()
            .map(|(component, bytes)| ErasedComponent {
                info: world.registry.get(component).expect("checked above"),
                payload: Payload::Bytes(bytes),
            })
            .collect();
        world.spawn_entity(keys[&id], region, components, stamp)?;
    }
    Ok(())
}

/// The `NetId` a stored key is exported under.
fn net_id(world: &World, key: StableKey) -> Result<NetId> {
    match key {
        StableKey::Net(id) => Ok(id),
        StableKey::Predicted(predicted) => world
            .alias(predicted)
            .ok_or_else(|| invalid(format!("{key:?} has no bound NetId to be exported under"))),
        StableKey::Placeholder(_) => Err(invalid("a placeholder reached storage")),
    }
}

pub(crate) fn export_region(world: &World, region: RegionId) -> Result<RegionRows> {
    let state = world
        .regions
        .get(&region)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "the region is not loaded"))?;
    let mut rows = Vec::new();
    for key in world.entities_in(region) {
        let loc = world.storage.loc(key).expect("a live entity");
        let archetype = &world.storage.archetypes[loc.archetype as usize];
        let chunk = &archetype.chunks[loc.chunk as usize];
        let mut owner = None;
        let mut components = Vec::new();
        let mut transient = Vec::new();
        for column in &archetype.columns {
            let bytes = chunk.row(column, loc.row as usize);
            if column.key == OwnedBy::KEY {
                if let Some(target) = typed::<OwnedBy>(bytes).owner.get() {
                    owner = Some(net_id(world, target)?);
                }
            } else if column.info.persistence() == Persistence::Persistent {
                components.push(ComponentData { component: column.key, bytes: bytes.to_vec() });
            } else if column.info.persistence() == Persistence::Transient {
                transient.push(column.key);
            }
        }
        rows.push(ExportedRow { id: net_id(world, key)?, owner, components, transient });
    }
    rows.sort_by_key(|row| row.id);
    let resources = state
        .resources
        .iter()
        .filter(|(_, resource)| resource.info.persistence() == Persistence::Persistent)
        .map(|(&component, resource)| ComponentData {
            component,
            bytes: resource.bytes.as_bytes().to_vec(),
        })
        .collect();
    Ok(RegionRows { seed: state.seed, resources, rows })
}
