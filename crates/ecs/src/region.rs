//! Regions: the unit of streaming, relevance and persistence (architecture section 6), and
//! the components the world itself maintains on every entity.

use bytemuck::{Pod, Zeroable};
use lactose_core::{EntityRef, FieldId, PlayerId, Tick};
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
    /// Per-entity and per-region systems run over it and commands may change its rows.
    Simulated,
    /// Rows are written only at the input sync point, from staged network state, through
    /// `&mut World`. No per-entity or per-region system runs over it.
    Replicated,
    /// Rows are read-only until the mode changes. No per-entity or per-region system runs
    /// over it.
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
/// [`World::move_to_region`](crate::World::move_to_region), and a system may read it. Not
/// saved: a region's record set implies it. Maintained: the schedule rejects `Mut` on it and
/// the direct methods reject inserting or removing it.
#[component(key = "ecs::in_region", derived)]
pub struct InRegion {
    /// The region the entity resides in.
    #[field(id = 1)]
    pub region: RegionId,
}

/// The owning entity (architecture section 6): an owned entity migrates with its owner
/// between regions and is despawned with it, and an
/// [`OwnershipChange`](lactose_bundle::Record::OwnershipChange) record carries the change
/// in a save. Written through [`Commands::set`](crate::Commands::set),
/// [`Commands::remove`](crate::Commands::remove) or the direct methods, never through
/// `Mut`, so the reverse index stays consistent.
#[component(key = "ecs::owned_by", persistent)]
pub struct OwnedBy {
    /// The owner, or the empty reference.
    #[field(id = 1)]
    pub owner: EntityRef,
}

/// The player whose input drives the entity: the player's own entity and everything it has
/// claimed (architecture section 9.4). Two uses:
///
/// - A spawn commanded from an entity with this component, by a system whose worlds include
///   both `server` and `prediction`, receives a
///   [`StableKey::Predicted`](lactose_core::StableKey::Predicted) origin key formed from
///   this player, the player's input tick this tick and an ordinal hashed from the
///   invocation ([`Commands::spawn`](crate::Commands::spawn)); the spawned entity inherits
///   the claim. The server and the client therefore derive the same key.
/// - The prediction world runs its per-entity systems only over rows that have it; game
///   code never writes that filter.
///
/// Session state: not saved, not hashed, not replicated. `net` and `predict` write it on
/// join through [`World::insert`](crate::World::insert) or
/// [`Commands::set`](crate::Commands::set) and release it with the matching `remove`; the
/// schedule rejects `Mut` on it. The input tick comes from
/// [`World::set_input_tick`](crate::World::set_input_tick) every tick.
#[component(key = "ecs::claimed_by", derived)]
pub struct ClaimedBy {
    /// The player.
    #[field(id = 1)]
    pub player: PlayerId,
}

/// The `Predicted` origin key of an entity the server bound a `NetId` to, so the origin key
/// survives a save and the region overlay a client receives (architecture sections 7 and
/// 15.5): RNG streams and command order use it on every machine. Written by the world when
/// it applies a predicted spawn and when it loads a row that carries it; never by a caller.
#[component(key = "ecs::predicted_origin", persistent)]
pub struct PredictedOrigin {
    /// The player whose input drove the spawn.
    #[field(id = 1)]
    pub player: PlayerId,
    /// The player's input tick at the spawn.
    #[field(id = 2)]
    pub input_tick: Tick,
    /// The spawn's ordinal: the low 32 bits of xxh64 over (source origin key, system key,
    /// spawn index within the invocation).
    #[field(id = 3)]
    pub ordinal: u32,
}
