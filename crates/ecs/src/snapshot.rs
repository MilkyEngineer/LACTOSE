//! Whole-world snapshots for the prediction world (architecture section 10).

use lactose_core::Tick;

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
    _private: (),
}

impl WorldSnapshot {
    /// The tick the snapshot was taken at.
    #[must_use]
    pub fn tick(&self) -> Tick {
        todo!()
    }

    /// The bytes the snapshot holds, which milestone 1 measures against its budget.
    #[must_use]
    pub fn size_in_bytes(&self) -> usize {
        todo!()
    }
}
