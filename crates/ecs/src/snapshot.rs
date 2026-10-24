//! Whole-world snapshots for the prediction world (architecture section 10).

use core::any::TypeId;
use core::num::NonZeroU64;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lactose_core::{Error, ErrorKind, NetId, PlayerId, PredictionKey, Result, StableKey, Tick};

use crate::region::RegionId;
use crate::storage::Storage;
use crate::world::{References, RegionState, ResourceEntry, World};

/// A copy of a world's rollback state at one tick: every column, derived ones included;
/// the entity table; every reflected world resource and region resource; the regions'
/// modes, seeds and asset tables; the input-tick table; and the relationship index. Not
/// service resources, the physics world, proxy history, command buffers or channels:
/// services are not state, physics has its own snapshot, proxy history is kept by
/// `lactose-predict`, and command buffers and channels are empty at a snapshot point by
/// construction, since the closing sync point of every run clears every channel (D126).
///
/// Two things a rollback must not undo stay with the world across a
/// [`restore`](crate::World::restore): the prediction-to-network alias table, since a bind
/// is a fact from the server, and the presentation-event log, since presentation has seen
/// what it has seen (architecture sections 7, 9.4 and 10).
///
/// Taken with [`World::snapshot`](crate::World::snapshot) and given back to
/// [`World::restore`](crate::World::restore) of the same world.
pub struct WorldSnapshot {
    identity: Arc<()>,
    tick: Tick,
    point: u32,
    next_net_id: NonZeroU64,
    storage: Storage,
    resources: Vec<(TypeId, ResourceEntry)>,
    regions: BTreeMap<RegionId, RegionState>,
    input_ticks: BTreeMap<PlayerId, Tick>,
    references: References,
    claims: BTreeMap<PlayerId, BTreeSet<StableKey>>,
}

impl WorldSnapshot {
    pub(crate) fn take(world: &World) -> Self {
        let resources = world
            .resources
            .sorted()
            .into_iter()
            .filter_map(|(&type_id, entry)| {
                let clone = entry.clone?;
                Some((
                    type_id,
                    ResourceEntry {
                        value: clone(&*entry.value),
                        clone: entry.clone,
                        size: entry.size,
                    },
                ))
            })
            .collect();
        Self {
            identity: world.identity.clone(),
            tick: world.tick,
            point: world.point,
            next_net_id: world.next_net_id,
            storage: world.storage.clone(),
            resources,
            regions: world.regions.clone(),
            input_ticks: world.input_ticks.clone(),
            references: world.references.clone(),
            claims: world.claims.clone(),
        }
    }

    pub(crate) fn restore_into(&self, world: &mut World) -> Result<()> {
        if !Arc::ptr_eq(&self.identity, &world.identity) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "the snapshot was taken from another world",
            ));
        }
        // Predicted entities live now and absent from the snapshot: the restore despawns
        // them, as of the snapshot's tick, so the rollback horizon prunes their aliases.
        let gone: Vec<PredictionKey> = world
            .storage
            .entities
            .keys()
            .filter_map(|key| match key {
                StableKey::Predicted(predicted) => Some(*predicted),
                _ => None,
            })
            .filter(|&predicted| {
                !self.storage.entities.contains_key(&StableKey::Predicted(predicted))
            })
            .collect();
        for predicted in gone {
            world.despawned_predicted.insert(predicted, self.tick);
        }
        world.tick = self.tick;
        world.point = self.point;
        world.point_cap = u32::MAX;
        world.next_net_id = self.next_net_id;
        world.storage = self.storage.clone();
        world.resources.retain(|_, entry| entry.clone.is_none());
        for (type_id, entry) in &self.resources {
            let clone = entry.clone.expect("only reflected resources are snapshotted");
            world.resources.insert(
                *type_id,
                ResourceEntry { value: clone(&*entry.value), clone: entry.clone, size: entry.size },
            );
        }
        world.regions = self.regions.clone();
        world.input_ticks = self.input_ticks.clone();
        world.references = self.references.clone();
        world.claims = self.claims.clone();
        // Binds survive a restore, so a reference the snapshot stored under a `Net` key that
        // has been bound since goes back to the origin key, as the bind left it.
        let rebinds: Vec<(NetId, PredictionKey)> = world
            .aliases
            .by_net
            .iter()
            // A `Net` entity the restore revived keeps the references that name it; the
            // revival is undone by `lactose-predict`, which despawns it again.
            .filter(|&(net, _)| !world.storage.entities.contains_key(&StableKey::Net(*net)))
            .map(|(&net, &predicted)| (net, predicted))
            .collect();
        // A rewrite that would close a cycle of owners (a revived entity owned by a `Net`
        // key bound since to an entity whose chain reaches it) empties that reference
        // instead (README, Regions, "Ownership is a forest").
        for (net, predicted) in rebinds {
            world.rekey_references(StableKey::Net(net), StableKey::Predicted(predicted));
        }
        world.events.clear();
        // Empty between runs, where a restore is made (D126); cleared all the same, as the
        // events are.
        world.channels.clear();
        world.present.rewind(self.tick);
        world.new_epoch();
        Ok(())
    }

    /// The tick the snapshot was taken at.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// The bytes the snapshot holds, which milestone 1 measures against its budget.
    #[must_use]
    pub fn size_in_bytes(&self) -> usize {
        let storage: usize = self
            .storage
            .archetypes
            .iter()
            .flat_map(|arch| &arch.chunks)
            .map(|chunk| {
                let stamps: usize = chunk
                    .stamps
                    .iter()
                    .flatten()
                    .map(|stamps| stamps.ticks.len() * (size_of::<Tick>() + size_of::<u32>()))
                    .sum();
                chunk.slab.as_bytes().len()
                    + chunk.keys.len() * size_of::<lactose_core::StableKey>()
                    + stamps
            })
            .sum();
        let resources: usize = self.resources.iter().map(|(_, entry)| entry.size).sum();
        let regions: usize = self
            .regions
            .values()
            .flat_map(|region| region.resources.values())
            .map(|resource| resource.bytes.as_bytes().len())
            .sum();
        storage + resources + regions
    }
}
