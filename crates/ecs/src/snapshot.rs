//! Whole-world snapshots for the prediction world (architecture section 10).

use core::any::TypeId;
use core::num::NonZeroU64;
use std::collections::BTreeMap;
use std::sync::Arc;

use lactose_core::{Error, ErrorKind, PlayerId, Result, Tick};

use crate::region::RegionId;
use crate::storage::Storage;
use crate::world::{References, RegionState, ResourceEntry, World};

/// A copy of a world's rollback state at one tick: every column, derived ones included;
/// the entity table; every reflected world resource and region resource; the regions'
/// modes, seeds and asset tables; the input-tick table; and the relationship index. Not
/// service resources, the physics world, proxy history or command buffers: services are
/// not state, physics has its own snapshot, proxy history is kept by `lactose-predict`, and
/// command buffers are empty at a snapshot point by construction.
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
        }
    }

    pub(crate) fn restore_into(&self, world: &mut World) -> Result<()> {
        if !Arc::ptr_eq(&self.identity, &world.identity) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "the snapshot was taken from another world",
            ));
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
        world.events.clear();
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
