//! Regions: the unit of streaming, relevance and persistence (architecture section 6), and
//! the components the world itself maintains on every entity.

use bytemuck::{Pod, Zeroable};
use lactose_core::{EntityRef, FieldId, PlayerId};
use lactose_reflect::{Field, FieldKind, FieldVisitor, ReferenceVisitor, component};

/// A region's identity. Stable across processes, the wire and saves: the cooker derives it
/// from the region's authored GUID, and the world never assigns one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Pod, Zeroable)]
#[repr(transparent)]
pub struct RegionId(pub u64);

impl Field for RegionId {
    const KIND: FieldKind = FieldKind::Unsigned;
    const NAME: &'static str = "RegionId";

    fn visit<V: FieldVisitor>(&self, id: FieldId, visitor: &mut V) {
        visitor.bytes(id, bytemuck::bytes_of(self));
    }

    fn visit_references<V: ReferenceVisitor>(&mut self, _: FieldId, _: &mut V) {}
}

/// What a region's rows do each tick (architecture section 6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RegionMode {
    /// Per-entity kernels run over its rows and commands may change them.
    Simulated,
    /// Rows are written only at the input sync point, from staged network state, through
    /// `&mut World`. No kernel runs over them.
    Replicated,
    /// Rows are read-only until the mode changes. No kernel runs over them.
    Frozen,
}

/// What [`World::create_region`](crate::World::create_region) needs.
#[derive(Clone, Copy, Debug)]
pub struct RegionConfig {
    /// The region.
    pub id: RegionId,
    /// The region seed, the first element of every RNG key drawn in the region (architecture
    /// section 8.2). Part of the save.
    pub seed: u64,
    /// The initial mode.
    pub mode: RegionMode,
}

/// What [`World::region`](crate::World::region) reports.
#[derive(Clone, Copy, Debug)]
pub struct RegionInfo {
    /// The region.
    pub id: RegionId,
    /// The region seed.
    pub seed: u64,
    /// The current mode.
    pub mode: RegionMode,
    /// How many entities reside in the region.
    pub entity_count: u32,
}

/// Region membership. Every entity has it; the world writes it at spawn, at
/// [`Commands::move_to_region`](crate::Commands::move_to_region) and at
/// [`World::move_to_region`](crate::World::move_to_region), and a system may read it. It is
/// not saved: a region's record set implies it.
#[component(key = "ecs::in_region", derived)]
pub struct InRegion {
    /// The region the entity resides in.
    #[field(id = 1)]
    pub region: RegionId,
}

/// The owning entity (architecture section 6): an owned entity migrates with its owner
/// between regions, and an [`OwnershipChange`](lactose_bundle::Record::OwnershipChange)
/// record carries the change in a save. Written through commands or `&mut World` like any
/// relationship, so the reverse index stays consistent.
#[component(key = "ecs::owned_by", persistent)]
pub struct OwnedBy {
    /// The owner, or the empty reference.
    #[field(id = 1)]
    pub owner: EntityRef,
}

/// The player whose input drives the entity: the player's own entity and everything it has
/// claimed (architecture section 9.4). Two uses:
///
/// - A spawn commanded from an entity with this component receives a
///   [`StableKey::Predicted`](lactose_core::StableKey::Predicted) origin key, formed from
///   this player, the player's current input tick and the command's ordinal among the
///   player's spawns this tick; the spawned entity inherits the component. The server and
///   the client therefore derive the same key.
/// - The prediction world runs its kernels only over rows that have it; game code never
///   writes that filter.
///
/// Session state: not saved, not hashed, not replicated. `net` and `predict` write it on
/// join; the input tick comes from [`World::set_input_tick`](crate::World::set_input_tick).
#[component(key = "ecs::claimed_by", derived)]
pub struct ClaimedBy {
    /// The player.
    #[field(id = 1)]
    pub player: PlayerId,
}
