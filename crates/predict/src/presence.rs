//! Transient presence on a claimed entity (`README.md`, "What a row built from the stream
//! has"; D141): which transient components a whole-entity statement says the entity has,
//! and how the hook and a correction leave the entity with them. Presence is never a
//! value: a component added is zeroed, and one the row has is never written.

use std::collections::BTreeSet;

use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, Result, StableKey};
use lactose_ecs::World;
use lactose_reflect::{Persistence, Registry, TypeInfo, TypeRole};

/// What a frame states of one entity's unreplicated transient components: the set its
/// `StagedFrame::transient` holds for the entity's key, its invalid keys already dropped
/// ([`unreplicated_transient`]), or `None` when it holds none.
pub(crate) type Statement = Option<BTreeSet<ComponentKey>>;

/// Whether `info` is a transient component the stream never sends as a value: the kind a
/// `StagedFrame::transient` statement names.
pub(crate) fn unreplicated_transient(info: &TypeInfo) -> bool {
    transient(info) && !info.tracking().replicated
}

fn transient(info: &TypeInfo) -> bool {
    info.role() == TypeRole::Component && info.persistence() == Persistence::Transient
}

/// The transient components a claimed spawn or a frame's exact rows (`components`) and the
/// frame's statement (`statement`) say the entity has: the replicated ones among
/// `components` and the statement's.
fn stated(
    registry: &Registry,
    components: &[ComponentData],
    statement: Option<&BTreeSet<ComponentKey>>,
) -> BTreeSet<ComponentKey> {
    let replicated = components.iter().map(|data| data.component).filter(|&component| {
        registry.get(component).is_some_and(|info| transient(info) && info.tracking().replicated)
    });
    replicated.chain(statement.into_iter().flatten().copied()).collect()
}

/// Appends to `spawn`, the components a row is spawned with, every transient component the
/// statement states that `spawn` lacks, zeroed, so the row is born in its final archetype
/// (rule 1). The all-zero value is valid for every transient type: registration refuses
/// one for which it is not.
pub(crate) fn born(
    registry: &Registry,
    spawn: &mut Vec<ComponentData>,
    components: &[ComponentData],
    statement: Option<&BTreeSet<ComponentKey>>,
) {
    for component in stated(registry, components, statement) {
        if spawn.iter().any(|data| data.component == component) {
            continue;
        }
        if let Some(info) = registry.get(component) {
            spawn.push(ComponentData { component, bytes: vec![0; info.layout().size()] });
        }
    }
}

/// Leaves the live `key` with the transient components the statement states, in ascending
/// component key order: one stated and absent inserted zeroed (rule 1), one stated and
/// present left as it is (rule 2), a replicated one `components` does not name removed
/// (rule 3), and, when the frame holds a statement, an unreplicated one it does not name
/// removed (rule 4). Whether a component was added or removed.
pub(crate) fn apply(
    world: &mut World,
    key: StableKey,
    components: &[ComponentData],
    statement: Option<&BTreeSet<ComponentKey>>,
) -> Result<bool> {
    let registry = world.registry();
    let stated = stated(registry, components, statement);
    // The registry iterates ascending by key: the order the writes reach the structural log.
    let ruled: Vec<(ComponentKey, usize)> = registry
        .iter()
        .filter(|info| transient(info) && (info.tracking().replicated || statement.is_some()))
        .map(|info| (info.key(), info.layout().size()))
        .collect();
    let mut changed = false;
    for (component, size) in ruled {
        let has = world.get_erased(key, component).is_some();
        if stated.contains(&component) {
            if !has {
                let zeroed = ComponentData { component, bytes: vec![0; size] };
                changed |= world.insert_erased(key, &zeroed)?;
            }
        } else if has {
            changed |= world.remove_erased(key, component)?;
        }
    }
    Ok(changed)
}
