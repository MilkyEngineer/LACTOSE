//! The facts of a staged frame (`README.md`, "Reconciliation"): what the server states
//! about an entity it has or had claimed for the local player, as the hook found them;
//! how one is compared with a ring slot's record; and what a correction writes for it.
//! Which components a correction writes is decided by [`writes`] alone, beside the
//! comparison it follows.

use std::collections::{BTreeMap, BTreeSet};

use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, Result, StableKey};
use lactose_ecs::{ClaimedBy, RegionId, World};
use lactose_reflect::{Persistence, Registry, TypeInfo, TypeRole};
use lactose_sim::StreamingAnchor;

use crate::compare::{Comparison, compare};
use crate::predictor::LocalPlayer;
use crate::ring::{EntityRecord, Slot};

/// One statement of a delta frame about an entity the server has or had claimed for the
/// local player, recorded by the hook in the frame's order (spawns, moves, removals, rows,
/// tombstones, each ascending by key) and never written by it.
#[derive(Clone, Debug)]
pub(crate) enum Fact {
    /// A spawn with no interpolation slot: the entity exists, claimed, in `region`, with
    /// these exact components.
    Claimed { key: StableKey, region: RegionId, components: Vec<ComponentData> },
    /// A spawn with a slot of an entity the prediction world had claimed: released, with
    /// these dequantised components.
    Released { key: StableKey, components: Vec<ComponentData> },
    /// The entity resides in `region`.
    Moved { key: StableKey, region: RegionId, claimed: bool },
    /// The entity lacks `component`.
    Removed { key: StableKey, component: ComponentKey, region: Option<RegionId>, claimed: bool },
    /// The entity's `component` has these exact bytes.
    Row {
        key: StableKey,
        component: ComponentKey,
        bytes: Vec<u8>,
        region: Option<RegionId>,
        claimed: bool,
    },
    /// The entity is gone.
    Destroyed { key: StableKey },
}

impl Fact {
    /// The entity the fact is about, by origin key.
    pub(crate) fn key(&self) -> StableKey {
        match *self {
            Self::Claimed { key, .. }
            | Self::Released { key, .. }
            | Self::Moved { key, .. }
            | Self::Removed { key, .. }
            | Self::Row { key, .. }
            | Self::Destroyed { key } => key,
        }
    }

    /// The fact with what the predictor knew when its frame was reconciled: whether it is
    /// about an entity the server has or had claimed (one the hook found claimed, or one
    /// in `known`), and the region it would be re-spawned in if a slot lacks it. `None` if
    /// it is about neither, such as the move of a proxy whose spawn was lost.
    pub(crate) fn resolve(self, known: &BTreeMap<StableKey, RegionId>) -> Option<Resolved> {
        let key = self.key();
        let region = match &self {
            Self::Claimed { region, .. } => Some(*region),
            Self::Released { .. } | Self::Destroyed { .. } => None,
            Self::Moved { region, claimed, .. } => {
                (*claimed || known.contains_key(&key)).then_some(())?;
                Some(*region)
            }
            Self::Removed { region, claimed, .. } | Self::Row { region, claimed, .. } => {
                (*claimed || known.contains_key(&key)).then_some(())?;
                known.get(&key).copied().or(*region)
            }
        };
        Some(Resolved { fact: self, region })
    }

    /// What reconciling the fact tells the predictor's map of claimed entities' regions.
    pub(crate) fn learn(
        &self,
        region: Option<RegionId>,
        known: &mut BTreeMap<StableKey, RegionId>,
    ) {
        match self {
            Self::Released { key, .. } | Self::Destroyed { key } => {
                known.remove(key);
            }
            Self::Claimed { key, region, .. } | Self::Moved { key, region, .. } => {
                known.insert(*key, *region);
            }
            Self::Removed { key, .. } | Self::Row { key, .. } => {
                if let Some(region) = region {
                    known.entry(*key).or_insert(region);
                }
            }
        }
    }
}

/// A fact the reconciliation acts on, with the region an absent entity is re-spawned in.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub(crate) fact: Fact,
    pub(crate) region: Option<RegionId>,
}

/// Whether the record holds the component `info` describes: a replicated persistent
/// component, the ones the server sends exact for a claimed entity. Derived and transient
/// ones are not sent.
pub(crate) fn recorded(info: &TypeInfo) -> bool {
    info.role() == TypeRole::Component
        && info.persistence() == Persistence::Persistent
        && info.tracking().replicated
}

/// How far a slot's record is from the facts for its tick: per entity that differs, how
/// many fields did. Empty when every fact agrees.
pub(crate) type Mismatches = BTreeMap<StableKey, u32>;

/// Compares `facts`, every fact for one tick, with `slot`, that tick's slot.
pub(crate) fn check(facts: &[Resolved], slot: &Slot, registry: &Registry) -> Mismatches {
    let mut mismatches = Mismatches::new();
    for resolved in facts {
        let key = resolved.fact.key();
        if let Some(fields) = mismatch(resolved, slot.record.get(&key), slot, registry) {
            *mismatches.entry(key).or_default() += fields;
        }
    }
    mismatches
}

/// How many fields of the fact differ if it does not match, `None` if it matches or is
/// skipped.
fn mismatch(
    resolved: &Resolved,
    entry: Option<&EntityRecord>,
    slot: &Slot,
    registry: &Registry,
) -> Option<u32> {
    let absent = || {
        let loaded = resolved.region.is_some_and(|region| slot.regions.contains(&region));
        if !loaded {
            tracing::debug!(
                target: "predict",
                key = ?resolved.fact.key(),
                tick = slot.tick.0,
                "a fact about an entity in a region the slot lacks is skipped"
            );
        }
        loaded.then_some(0)
    };
    match &resolved.fact {
        Fact::Claimed { region, components, .. } => {
            let Some(entry) = entry else { return absent() };
            let mut differs = entry.region != *region;
            let mut fields = 0;
            for data in components {
                match compare_data(registry, entry, data.component, &data.bytes) {
                    Some(Comparison::Differs { fields: n }) => {
                        differs = true;
                        fields += n;
                    }
                    Some(Comparison::Missing) => differs = true,
                    _ => {}
                }
            }
            differs.then_some(fields)
        }
        Fact::Released { .. } | Fact::Destroyed { .. } => entry.map(|_| 0),
        Fact::Moved { region, .. } => match entry {
            Some(entry) => (entry.region != *region).then_some(0),
            None => absent(),
        },
        Fact::Removed { component, .. } => match entry {
            Some(entry) => entry.components.contains_key(component).then_some(0),
            None => absent(),
        },
        Fact::Row { component, bytes, .. } => {
            let Some(entry) = entry else { return absent() };
            match compare_data(registry, entry, *component, bytes)? {
                Comparison::Differs { fields } => Some(fields),
                Comparison::Missing => Some(0),
                _ => None,
            }
        }
    }
}

/// The comparison of one component of a fact with the record, or `None` when the sender
/// named a component the registry or the record does not hold, which is logged.
fn compare_data(
    registry: &Registry,
    entry: &EntityRecord,
    component: ComponentKey,
    bytes: &[u8],
) -> Option<Comparison> {
    let Some(info) = registry.get(component).filter(|info| recorded(info)) else {
        tracing::warn!(
            target: "predict",
            component = component.0,
            "a fact names a component that is not a registered replicated one: skipped"
        );
        return None;
    };
    let comparison = compare(info, entry.components.get(&component).map(Vec::as_slice), bytes);
    if comparison == Comparison::Malformed {
        tracing::warn!(
            target: "predict",
            component = info.name(),
            "a fact's bytes are not the component's size: skipped"
        );
        return None;
    }
    Some(comparison)
}

/// Whether a correction writes a component after `comparison`: where the world differs
/// from the server or lacks the component. The one place that decides which of a tick's
/// facts a correction writes.
fn writes(comparison: Comparison) -> bool {
    matches!(comparison, Comparison::Differs { .. } | Comparison::Missing)
}

/// Writes every fact of `facts` into `world`, which is at their tick, in their order, each
/// write stamped: the correction of `README.md`, "Reconciliation", step 2. Returns the
/// entities whose rows, presence, region or claim changed, in ascending order. A write the
/// world refuses is logged and skipped.
pub(crate) fn write(
    world: &mut World,
    local: LocalPlayer,
    facts: &[Resolved],
) -> BTreeSet<StableKey> {
    let mut written = BTreeSet::new();
    for resolved in facts {
        let key = resolved.fact.key();
        match write_fact(world, local, resolved) {
            Ok(true) => {
                written.insert(key);
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(target: "predict", ?key, %error, "a correction was refused: skipped");
            }
        }
    }
    written
}

fn write_fact(world: &mut World, local: LocalPlayer, resolved: &Resolved) -> Result<bool> {
    let key = resolved.fact.key();
    let live = world.contains(key);
    match &resolved.fact {
        Fact::Claimed { region, components, .. } => {
            if !live {
                let components = written_components(world, None, components);
                return respawn(world, local, key, Some(*region), &components);
            }
            let mut changed = claim(world, local, key)?;
            changed |= move_to(world, key, *region)?;
            for data in written_components(world, Some(key), components) {
                changed |= world.insert_erased(key, &data)?;
            }
            Ok(changed)
        }
        Fact::Released { components, .. } => {
            if !live {
                return Ok(false);
            }
            let mut changed = false;
            if world.get::<ClaimedBy>(key).is_some_and(|claim| claim.player == local.player) {
                world.remove::<ClaimedBy>(key)?;
                changed = true;
            }
            for data in written_components(world, Some(key), components) {
                changed |= world.insert_erased(key, &data)?;
            }
            Ok(changed)
        }
        Fact::Moved { region, .. } => {
            if !live {
                return respawn(world, local, key, Some(*region), &[]);
            }
            Ok(claim(world, local, key)? | move_to(world, key, *region)?)
        }
        Fact::Removed { component, .. } => {
            if !live {
                return respawn(world, local, key, resolved.region, &[]);
            }
            Ok(claim(world, local, key)? | world.remove_erased(key, *component)?)
        }
        Fact::Row { component, bytes, .. } => {
            let data = [ComponentData { component: *component, bytes: bytes.clone() }];
            if !live {
                let data = written_components(world, None, &data);
                return respawn(world, local, key, resolved.region, &data);
            }
            let mut changed = claim(world, local, key)?;
            for data in written_components(world, Some(key), &data) {
                changed |= world.insert_erased(key, &data)?;
            }
            Ok(changed)
        }
        Fact::Destroyed { .. } => {
            if live {
                world.despawn(key)?;
            }
            Ok(live)
        }
    }
}

/// The components of `components` a correction writes to `key` (`None`: to an entity it
/// spawns), by [`writes`] against what the world holds.
fn written_components(
    world: &World,
    key: Option<StableKey>,
    components: &[ComponentData],
) -> Vec<ComponentData> {
    let registry = world.registry();
    components
        .iter()
        .filter(|data| {
            registry.get(data.component).filter(|info| recorded(info)).is_some_and(|info| {
                let stored = key.and_then(|key| world.get_erased(key, data.component));
                writes(compare(info, stored, &data.bytes))
            })
        })
        .cloned()
        .collect()
}

/// Spawns `key` claimed for the local player in `region` with `components`, and the anchor
/// if it is the player's own entity; skipped and logged when the region is unknown or not
/// loaded.
fn respawn(
    world: &mut World,
    local: LocalPlayer,
    key: StableKey,
    region: Option<RegionId>,
    components: &[ComponentData],
) -> Result<bool> {
    let Some(region) = region.filter(|&region| world.region(region).is_some()) else {
        tracing::debug!(
            target: "predict",
            ?key,
            "an entity to re-spawn has no loaded region: skipped"
        );
        return Ok(false);
    };
    world.spawn_erased(key, region, components)?;
    world.insert(key, ClaimedBy { player: local.player })?;
    if world.origin(local.entity) == Some(key) {
        world.insert(key, StreamingAnchor { priority: 0 })?;
    }
    Ok(true)
}

/// Gives `key` the local player's claim if it lacks it; whether it wrote.
fn claim(world: &mut World, local: LocalPlayer, key: StableKey) -> Result<bool> {
    if world.get::<ClaimedBy>(key).is_some_and(|claim| claim.player == local.player) {
        return Ok(false);
    }
    world.insert(key, ClaimedBy { player: local.player })?;
    Ok(true)
}

/// Moves `key` to `region` if it resides elsewhere; whether it moved.
fn move_to(world: &mut World, key: StableKey, region: RegionId) -> Result<bool> {
    if world.region_of(key) == Some(region) {
        return Ok(false);
    }
    world.move_to_region(key, region)?;
    Ok(true)
}
