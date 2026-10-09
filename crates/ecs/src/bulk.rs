//! Bulk copy of region rows to and from `lactose_bundle` documents (architecture sections
//! 15.2 and 15.4).

use lactose_bundle::ComponentData;
use lactose_core::NetId;

/// Every persistent component of one entity, as [`World::export_region`](crate::World::export_region)
/// copies it out. `EntityRef` fields hold `Net` keys: a reference to a predicted entity is
/// written as its bound `NetId`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExportedRow {
    /// The entity's `NetId`.
    pub id: NetId,
    /// The owning entity, if any.
    pub owner: Option<NetId>,
    /// The persistent components, in ascending key order, each exactly as the component is
    /// laid out in memory.
    pub components: Vec<ComponentData>,
}

/// The rows of one region, in ascending `NetId` order, so two exports of equal state are
/// equal whatever the layout. `lactose-persist` diffs them against the cooked baseline
/// into an [`Overlay`](lactose_bundle::Overlay); the same records serve a save, an unload
/// and what the server sends a client when the region becomes relevant.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RegionRows {
    /// The rows.
    pub rows: Vec<ExportedRow>,
}
