//! The facts of a staged frame (`README.md`, "Reconciliation"): what the server states
//! about an entity it has or had claimed for the local player, as the hook found them;
//! how one is compared with a ring slot's record; and what a correction writes for it.
//! Which components a correction writes is decided by [`writes`] alone, beside the
//! comparison it follows.

use std::collections::{BTreeMap, BTreeSet};

use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, Result, StableKey};
use lactose_ecs::{ClaimedBy, RegionId, World};
use lactose_physics_api::{BodyType, RigidBody, Teleported};
use lactose_reflect::{Persistence, Reflect, Registry, TypeInfo, TypeRole, decode};
use lactose_sim::StreamingAnchor;

use crate::compare::{Comparison, PredictionTolerance, compare};
use crate::predictor::LocalPlayer;
use crate::reconcile::Absorbed;
use crate::ring::{EntityRecord, Slot};
use crate::staged_region::place_residence;

/// One statement of a delta frame about an entity the server has or had claimed for the
/// local player, recorded by the hook in the frame's order (spawns, moves, removals, rows,
/// tombstones, each ascending by key); of these it writes only a claim or a release, in a
/// predicted tick.
#[derive(Clone, Debug)]
pub(crate) enum Fact {
    /// A spawn with no interpolation slot: the entity exists, claimed, in `region`, and its
    /// recorded components are exactly these, with these exact bytes.
    Claimed { key: StableKey, region: RegionId, components: Vec<ComponentData> },
    /// A spawn with a slot of an entity the prediction world had claimed: released, in
    /// `region` (the server's, which the proxy's `StagedRegion` takes), with these
    /// dequantised components.
    Released { key: StableKey, region: RegionId, components: Vec<ComponentData> },
    /// The entity resides in `region`.
    Moved { key: StableKey, region: RegionId, claimed: bool },
    /// The entity lacks `component`.
    Removed { key: StableKey, component: ComponentKey, region: Option<RegionId>, claimed: bool },
    /// The frame's exact rows of the entity, together, ascending by component: its
    /// recorded components are exactly these, with these bytes, since the server sends a
    /// claimed entity whole in every frame that carries a row of it (D121).
    Rows { key: StableKey, components: Vec<ComponentData>, region: Option<RegionId>, claimed: bool },
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
            | Self::Rows { key, .. }
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
            Self::Removed { region, claimed, .. } | Self::Rows { region, claimed, .. } => {
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
            Self::Removed { key, .. } | Self::Rows { key, .. } => {
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

/// Adds `new`, a later fact about an entity, to `kept`, the facts compared at one tick,
/// so that each entity's facts there stay one statement a record can agree with: what
/// `new` states replaces what it supersedes. A claimed spawn, release or tombstone replaces
/// the entity's earlier facts; a move sets the region they name; exact rows replace a
/// claimed spawn's components, or earlier rows and removals; a removal drops the component
/// from a claimed spawn or rows. Whether `new` is kept, which a move, removal or exact rows
/// after a release or tombstone of the entity are not: the server states nothing else about
/// one it no longer claims.
pub(crate) fn fold(kept: &mut Vec<Resolved>, new: Resolved) -> bool {
    let key = new.fact.key();
    let ours = |resolved: &Resolved| resolved.fact.key() == key;
    let ended = kept.iter().any(|resolved| {
        ours(resolved) && matches!(resolved.fact, Fact::Released { .. } | Fact::Destroyed { .. })
    });
    // The claimed spawn or exact rows that state the entity whole, if one is kept.
    let whole = kept.iter().position(|resolved| {
        ours(resolved) && matches!(resolved.fact, Fact::Claimed { .. } | Fact::Rows { .. })
    });
    match &new.fact {
        Fact::Claimed { .. } | Fact::Released { .. } | Fact::Destroyed { .. } => {
            kept.retain(|resolved| !ours(resolved));
        }
        _ if ended => return false,
        Fact::Moved { region, .. } => {
            for resolved in kept.iter_mut().filter(|resolved| ours(resolved)) {
                resolved.region = Some(*region);
                if let Fact::Claimed { region: stated, .. } = &mut resolved.fact {
                    *stated = *region;
                }
            }
            if whole.is_some_and(|at| matches!(kept[at].fact, Fact::Claimed { .. })) {
                return true;
            }
            kept.retain(|resolved| {
                !(ours(resolved) && matches!(resolved.fact, Fact::Moved { .. }))
            });
        }
        Fact::Rows { components, .. } => {
            if let Some(at) = whole
                && let Fact::Claimed { components: stated, .. } = &mut kept[at].fact
            {
                stated.clone_from(components);
                return true;
            }
            kept.retain(|resolved| {
                !(ours(resolved)
                    && matches!(resolved.fact, Fact::Rows { .. } | Fact::Removed { .. }))
            });
        }
        Fact::Removed { component, .. } => {
            if let Some(at) = whole {
                if let Fact::Claimed { components, .. } | Fact::Rows { components, .. } =
                    &mut kept[at].fact
                {
                    components.retain(|data| data.component != *component);
                }
                return true;
            }
            kept.retain(|resolved| {
                !(ours(resolved)
                    && matches!(resolved.fact, Fact::Removed { component: c, .. } if c == *component))
            });
        }
    }
    kept.push(new);
    true
}

/// Whether the record holds the component `info` describes: a replicated persistent
/// component, which the server sends exact for a claimed entity, other than `Teleported`,
/// the client's own no-interpolate flag (D124). A transient one the server may send is
/// zeroed by the next sync point before anything reads it; a derived one it never sends.
pub(crate) fn recorded(info: &TypeInfo) -> bool {
    info.role() == TypeRole::Component
        && info.persistence() == Persistence::Persistent
        && info.tracking().replicated
        && info.key() != Teleported::KEY
}

/// The recorded components of `components`: what a whole-entity statement names.
fn named(registry: &Registry, components: &[ComponentData]) -> BTreeSet<ComponentKey> {
    components
        .iter()
        .map(|data| data.component)
        .filter(|&component| registry.get(component).is_some_and(recorded))
        .collect()
}

/// How far a slot's record is from the facts for its tick: per entity that differs, how
/// many fields did. Empty when every fact agrees.
pub(crate) type Mismatches = BTreeMap<StableKey, u32>;

/// What comparing one tick's facts with its slot found.
#[derive(Default)]
pub(crate) struct Checked {
    /// Every entity a fact did not agree with.
    pub(crate) mismatches: Mismatches,
    /// What the tolerance absorbed.
    pub(crate) absorbed: Absorbed,
}

/// How facts are judged: the world's registry and the predictor's tolerance.
pub(crate) struct Judge<'a> {
    pub(crate) registry: &'a Registry,
    pub(crate) tolerance: &'a PredictionTolerance,
}

/// Compares `facts`, every fact for one tick, with `slot`, that tick's slot; `before` is
/// the slot of the tick before it if the ring holds one, for a body's type at *T* - 1.
pub(crate) fn check(
    facts: &[Resolved],
    slot: &Slot,
    before: Option<&Slot>,
    judge: &Judge<'_>,
) -> Checked {
    let mut checked = Checked::default();
    for resolved in facts {
        let key = resolved.fact.key();
        let entry = slot.record.get(&key);
        let previous = before.and_then(|slot| slot.record.get(&key));
        let found = mismatch(resolved, entry, previous, slot, judge, &mut checked.absorbed);
        if let Some(fields) = found {
            *checked.mismatches.entry(key).or_default() += fields;
        }
    }
    checked
}

/// How many fields of the fact differ if it does not match, `None` if it matches or is
/// skipped.
fn mismatch(
    resolved: &Resolved,
    entry: Option<&EntityRecord>,
    previous: Option<&EntityRecord>,
    slot: &Slot,
    judge: &Judge<'_>,
    absorbed: &mut Absorbed,
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
            let found = statement(components, entry, previous, judge, absorbed);
            // A region the slot lacks is a sender defect, logged when the frame was
            // reconciled: a correction could not place the entity there.
            if entry.region == *region || !slot.regions.contains(region) {
                found
            } else {
                Some(found.unwrap_or(0))
            }
        }
        Fact::Released { .. } | Fact::Destroyed { .. } => entry.map(|_| 0),
        Fact::Moved { region, .. } => match entry {
            Some(entry) => (entry.region != *region && slot.regions.contains(region)).then_some(0),
            None => absent(),
        },
        Fact::Removed { component, .. } => match entry {
            Some(entry) => entry.components.contains_key(component).then_some(0),
            None => absent(),
        },
        // Rows of no recorded component (`Teleported` alone, a transient one) state nothing.
        Fact::Rows { components, .. } if named(judge.registry, components).is_empty() => None,
        Fact::Rows { components, .. } => {
            let Some(entry) = entry else { return absent() };
            statement(components, entry, previous, judge, absorbed)
        }
    }
}

/// How many fields of a whole-entity statement (a claimed spawn, a frame's exact rows)
/// differ from the entity's record if it does not match: a component that differs, one
/// the record lacks, and one the record holds that the statement does not name, the last
/// two with no field. What the tolerance absorbed is added to `absorbed`, and agrees.
fn statement(
    components: &[ComponentData],
    entry: &EntityRecord,
    previous: Option<&EntityRecord>,
    judge: &Judge<'_>,
    absorbed: &mut Absorbed,
) -> Option<u32> {
    let tolerance = dynamic(components, entry, previous).then_some(judge.tolerance);
    let mut differs = false;
    let mut fields = 0;
    for data in components {
        match compare_data(judge.registry, entry, data, tolerance) {
            Some(Comparison::Differs { fields: n }) => {
                differs = true;
                fields += n;
            }
            Some(Comparison::Missing) => differs = true,
            Some(Comparison::Absorbed(within)) => absorbed.add(&within),
            _ => {}
        }
    }
    let named = named(judge.registry, components);
    differs |= entry.components.keys().any(|component| !named.contains(component));
    differs.then_some(fields)
}

/// Whether the entity's `RigidBody` is `Dynamic` at the compared slot (the statement's if
/// it names one, else the record's) or at the slot before (that record's): the condition
/// of the tolerance. The compared slot alone decides when the ring holds no slot before
/// it, or the entity was not claimed there.
fn dynamic(
    components: &[ComponentData],
    entry: &EntityRecord,
    previous: Option<&EntityRecord>,
) -> bool {
    let stated = components.iter().find(|data| data.component == RigidBody::KEY);
    let now = stated
        .map(|data| data.bytes.as_slice())
        .or_else(|| entry.components.get(&RigidBody::KEY).map(Vec::as_slice));
    let before = previous.and_then(|entry| entry.components.get(&RigidBody::KEY));
    [now, before.map(Vec::as_slice)].into_iter().flatten().any(|bytes| {
        decode::<RigidBody>(bytes).is_ok_and(|body| body.body_type == BodyType::Dynamic)
    })
}

/// The comparison of one component of a fact with the record, or `None` when it is not
/// compared: a replicated component the record never holds (a transient one, `Teleported`)
/// is skipped silently; one the registry lacks or holds as unreplicated, and bytes that are
/// not one value, are defects of the sender, logged and skipped.
fn compare_data(
    registry: &Registry,
    entry: &EntityRecord,
    data: &ComponentData,
    tolerance: Option<&PredictionTolerance>,
) -> Option<Comparison> {
    let info = registry.get(data.component);
    let Some(info) = info.filter(|info| recorded(info)) else {
        let replicated = info
            .is_some_and(|info| info.role() == TypeRole::Component && info.tracking().replicated);
        if !replicated {
            tracing::warn!(
                target: "predict",
                component = data.component.0,
                "a fact names a component that is not a registered replicated one: skipped"
            );
        }
        return None;
    };
    let predicted = entry.components.get(&data.component).map(Vec::as_slice);
    let comparison = compare(info, predicted, &data.bytes, tolerance);
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
/// from the server by bytes, within the tolerance or not, or lacks the component. A
/// correction writes every fact at its tick, the absorbed ones included, since writing them
/// is what converges a dynamic body (D124). The one place that decides which of a tick's
/// facts a correction writes.
fn writes(comparison: Comparison) -> bool {
    matches!(comparison, Comparison::Differs { .. } | Comparison::Absorbed(_) | Comparison::Missing)
}

/// Writes every fact of `facts` into `world`, which is at their tick, in their order, each
/// write stamped: the correction of `README.md`, "Reconciliation", step 2. `recorded` is
/// every recorded component, ascending: what a whole-entity statement removes when it does
/// not name it. Returns the entities whose rows, presence, region or claim changed, in
/// ascending order. A write the world refuses is logged and skipped.
pub(crate) fn write(
    world: &mut World,
    local: LocalPlayer,
    recorded: &[ComponentKey],
    facts: &[Resolved],
) -> BTreeSet<StableKey> {
    let mut written = BTreeSet::new();
    for resolved in facts {
        let key = resolved.fact.key();
        match write_fact(world, local, recorded, resolved) {
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

fn write_fact(
    world: &mut World,
    local: LocalPlayer,
    recorded: &[ComponentKey],
    resolved: &Resolved,
) -> Result<bool> {
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
            changed |= insert(world, key, components)?;
            changed |= strip(world, key, recorded, components)?;
            Ok(changed)
        }
        // As the hook applies a release in a predicted tick: the claim removed, the entity
        // moved where the releasing spawn has it, its components upserted and where that
        // left it recorded. The move holds the proxy where the server released it during
        // the replayed ticks between this one and the tick the releasing frame was popped
        // at, where the replay's upsert of that frame restates it (a replay never
        // re-applies the claim change itself).
        Fact::Released { region, components, .. } => {
            if !live {
                return Ok(false);
            }
            let mut changed = false;
            if world.get::<ClaimedBy>(key).is_some_and(|claim| claim.player == local.player) {
                world.remove::<ClaimedBy>(key)?;
                changed = true;
            }
            changed |= move_to(world, key, *region)?;
            changed |= insert(world, key, components)?;
            place_residence(world, key)?;
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
        Fact::Rows { components, .. } => {
            if named(world.registry(), components).is_empty() {
                return Ok(false);
            }
            if !live {
                let components = written_components(world, None, components);
                return respawn(world, local, key, resolved.region, &components);
            }
            let mut changed = claim(world, local, key)?;
            changed |= insert(world, key, components)?;
            changed |= strip(world, key, recorded, components)?;
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

/// Writes to the live `key` the components of `components` a correction writes; whether
/// one changed.
fn insert(world: &mut World, key: StableKey, components: &[ComponentData]) -> Result<bool> {
    let mut changed = false;
    for data in written_components(world, Some(key), components) {
        changed |= world.insert_erased(key, &data)?;
    }
    Ok(changed)
}

/// Removes from the live `key` every component of `recorded` it has that the whole-entity
/// statement `components` does not name; whether one was removed.
fn strip(
    world: &mut World,
    key: StableKey,
    recorded: &[ComponentKey],
    components: &[ComponentData],
) -> Result<bool> {
    let named = named(world.registry(), components);
    let mut changed = false;
    for &component in recorded {
        if !named.contains(&component) && world.get_erased(key, component).is_some() {
            changed |= world.remove_erased(key, component)?;
        }
    }
    Ok(changed)
}

/// The components of `components` a correction writes to `key` (`None`: to an entity it
/// spawns), by [`writes`] against what the world holds.
pub(crate) fn written_components(
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
                writes(compare(info, stored, &data.bytes, None))
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

/// Moves `key` to `region` if it resides elsewhere and the world holds `region`; whether it
/// moved. A region the world lacks is logged and skipped, so the rest of a claimed spawn's
/// statement is still written.
fn move_to(world: &mut World, key: StableKey, region: RegionId) -> Result<bool> {
    if world.region_of(key) == Some(region) {
        return Ok(false);
    }
    if world.region(region).is_none() {
        tracing::debug!(target: "predict", ?key, ?region, "a move into a region not loaded: skipped");
        return Ok(false);
    }
    world.move_to_region(key, region)?;
    Ok(true)
}
