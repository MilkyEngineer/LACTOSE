//! Bulk copy of region rows to and from `lactose_bundle` documents (architecture sections
//! 15.2 and 15.4).

use lactose_bundle::ComponentData;
use lactose_core::NetId;

/// Every persistent component of one entity, as [`World::export_region`](crate::World::export_region)
/// copies it out. `EntityRef` fields hold `Net` keys: a reference to a predicted entity is
/// written as its bound `NetId`. Bytes are copied as laid out in memory and not
/// canonicalised; `lactose-persist` canonicalises scalars when it serialises through
/// `lactose-reflect`, and a NaN in a persistent column is a debug assertion before that
/// (architecture section 8.2).
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
