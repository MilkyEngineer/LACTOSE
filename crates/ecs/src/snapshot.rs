//! Whole-world snapshots for the prediction world (architecture section 10).

use lactose_core::Tick;

/// A copy of a world's rollback state at one tick: mutable columns, the entity table, the
/// prediction-to-network alias table, region resources, the relationship index, the
/// presentation event log and the regions' modes and seeds. Not the physics world, proxy
/// history or command buffers: physics has its own snapshot, proxy history is kept by
/// `lactose-predict`, and command buffers are empty at a snapshot point by construction.
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
