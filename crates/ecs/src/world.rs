//! The world: entities, components, resources and regions with their change state
//! (architecture section 6), and the direct access that exclusive systems and engine
//! passes use between stages.

use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::sync::Arc;

use lactose_asset_api::PinnedTable;
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{
    ComponentKey, FieldId, NetId, PlayerId, PredictionKey, Result, StableKey, SystemKey, Tick,
};
use lactose_reflect::{Component, Registry, Resource};

use crate::bulk::RegionRows;
use crate::change::{
    ChangeMark, ChangedRows, Columns, ColumnsMut, ErasedColumns, LogCursor, StructuralLog,
};
use crate::command::{Bundle, ChannelEntry, CommandChannel};
use crate::event::PresentEventBatch;
use crate::param::Mut;
use crate::query::{Query, QueryItems};
use crate::region::{RegionConfig, RegionId, RegionInfo, RegionMode};
use crate::rng::Rng;
use crate::snapshot::WorldSnapshot;

/// Which world this is. A system declares the kinds it runs in, and the world applies its
/// own filter to every kernel (architecture section 9.4), so game code never writes one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WorldKind {
    /// The authoritative world. Spawns receive runtime `NetId`s.
    Server,
    /// A client's replicated world: rows hold received state, written at the input sync
    /// point. It runs no per-entity kernel (architecture section 5.3).
    Client,
    /// A client's prediction world: kernels run only over rows that have
    /// [`ClaimedBy`](crate::ClaimedBy), spawns receive `Predicted` keys, and it is the one
    /// world that is snapshotted and restored.
    Prediction,
}

/// A set of world kinds: what `worlds = [server, prediction]` denotes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WorldSet(u8);

impl WorldSet {
    /// No world.
    pub const NONE: Self = Self(0);
    /// The server world.
    pub const SERVER: Self = Self(1);
    /// The client's replicated world.
    pub const CLIENT: Self = Self(2);
    /// The client's prediction world.
    pub const PREDICTION: Self = Self(4);
    /// Every world, which is what a system without `worlds` declares.
    pub const ALL: Self = Self(7);

    /// Both sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether `kind` is in the set.
    #[must_use]
    pub const fn contains(self, kind: WorldKind) -> bool {
        let bit = match kind {
            WorldKind::Server => Self::SERVER.0,
            WorldKind::Client => Self::CLIENT.0,
            WorldKind::Prediction => Self::PREDICTION.0,
        };
        self.0 & bit != 0
    }
}

/// The order chunk jobs are issued in: a layout-mode option of SOUR (architecture section
/// 8.3), a no-op for results.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChunkOrder {
    /// Storage order.
    #[default]
    Natural,
    /// Reversed.
    Reversed,
    /// Shuffled by the layout seed.
    Shuffled,
}

/// The storage-layout options SOUR varies and production leaves at the default. Runtime
/// options, never `cfg(test)` or Cargo features (architecture section 8.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LayoutConfig {
    /// Rows per chunk for every archetype, or `None` to derive it from the row size within
    /// a 16 KiB slab. The one-row capacity uses a matching small slab.
    pub chunk_rows: Option<u32>,
    /// The seed for row permutation, chunk shuffling and batch splitting.
    pub seed: u64,
    /// Whether rows are permuted within their chunks at every sync point, identities
    /// preserved.
    pub permute_rows: bool,
    /// How chunk jobs are ordered.
    pub chunk_order: ChunkOrder,
    /// Whether chunk jobs are split into batches of random size.
    pub split_batches: bool,
}

/// Builds a [`World`].
pub struct WorldBuilder {
    kind: WorldKind,
    registry: Arc<Registry>,
    layout: LayoutConfig,
    tick: Tick,
    next_net_id: NonZeroU64,
}

impl WorldBuilder {
    /// A world of `kind` whose components and resources are the types in `registry`.
    #[must_use]
    pub fn new(kind: WorldKind, registry: Arc<Registry>) -> Self {
        todo!()
    }

    /// The storage layout; the default is production.
    #[must_use]
    pub fn layout(mut self, layout: LayoutConfig) -> Self {
        self.layout = layout;
        self
    }

    /// The tick the world starts at: the server's tick on a client, the saved tick on a
    /// load. The default is zero.
    #[must_use]
    pub fn tick(mut self, tick: Tick) -> Self {
        self.tick = tick;
        self
    }

    /// The counter the next runtime `NetId` is allocated from, restored from a save
    /// (architecture section 7). The default starts a fresh server.
    #[must_use]
    pub fn next_net_id(mut self, counter: NonZeroU64) -> Self {
        self.next_net_id = counter;
        self
    }

    /// Builds the world. Fails if the registry lacks the components the world itself
    /// maintains ([`InRegion`](crate::InRegion), [`OwnedBy`](crate::OwnedBy),
    /// [`ClaimedBy`](crate::ClaimedBy), [`PredictedOrigin`](crate::PredictedOrigin)) or if a
    /// chunk capacity cannot hold a registered component.
    pub fn build(self) -> Result<World> {
        todo!()
    }
}

/// Entities, components, resources and regions, with their change state.
///
/// A `World` is owned by the tick loop that runs its [`Schedule`](crate::Schedule), which
/// takes it by value for a tick and hands it back. Between ticks the methods here give
/// exclusive code direct access; during a stage the kernels see it through their
/// parameters. Every method that takes a [`StableKey`] accepts the origin key or, for a
/// predicted entity the server has bound, its `Net` key.
///
/// `World` is `Send` and `Sync`: a scope child of a [`Job`](crate::Job) may borrow it.
pub struct World {
    _private: (),
}

impl World {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        todo!()
    }

    /// The registry the world was built with.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        todo!()
    }

    /// The layout applied, for a SOUR run to record.
    #[must_use]
    pub fn layout(&self) -> &LayoutConfig {
        todo!()
    }

    /// The absolute simulation tick: the tick being run, or the last one run between ticks.
    /// It is never rebased; RNG keys and tick-valued fields use it. Stored change stamps
    /// are rebased every 2^30 ticks and at a restore or a whole-world load, which starts a
    /// new epoch for every [`ChangeMark`] and [`LogCursor`].
    #[must_use]
    pub fn tick(&self) -> Tick {
        todo!()
    }

    /// Sets the tick between ticks: a client re-syncing to the server's clock, a server
    /// after a load. Starts a new epoch.
    pub fn set_tick(&mut self, tick: Tick) -> Result<()> {
        todo!()
    }

    /// A mark at the current tick and epoch.
    #[must_use]
    pub fn mark(&self) -> ChangeMark {
        todo!()
    }

    /// Whether `mark` is of the current epoch, so change stamps compare against its tick.
    #[must_use]
    pub fn is_current(&self, mark: ChangeMark) -> bool {
        todo!()
    }

    // ---- Entities ----

    /// Whether `key` names a live entity.
    #[must_use]
    pub fn contains(&self, key: StableKey) -> bool {
        todo!()
    }

    /// The origin key of `key`'s entity: `key` itself, or the `Predicted` key behind a
    /// bound `Net` key. `None` if the entity does not exist.
    #[must_use]
    pub fn origin(&self, key: StableKey) -> Option<StableKey> {
        todo!()
    }

    /// The region `key` resides in.
    #[must_use]
    pub fn region_of(&self, key: StableKey) -> Option<RegionId> {
        todo!()
    }

    /// Component `T` of `key`, or `None` if the entity does not exist or lacks it.
    #[must_use]
    pub fn get<T: Component>(&self, key: StableKey) -> Option<&T> {
        todo!()
    }

    /// Component `T` of `key` for writing, marking the row changed on a mutable
    /// dereference, like the kernel parameter. Panics for a component the world maintains
    /// or one with an `EntityRef` field; those go through [`insert`](Self::insert).
    #[must_use]
    pub fn get_mut<T: Component>(&mut self, key: StableKey) -> Option<Mut<'_, T>> {
        todo!()
    }

    /// Spawns an entity with `key` in `region`, immediately. `key` is a `Net` or
    /// `Predicted` key that no live entity has; a placeholder is an error. Replication and
    /// prediction spawn this way with the server's key; a kernel spawns through
    /// [`Commands::spawn`](crate::Commands::spawn), which assigns the key. A `Predicted`
    /// key gives the entity [`PredictedOrigin`](crate::PredictedOrigin).
    pub fn spawn<B: Bundle>(&mut self, key: StableKey, region: RegionId, bundle: B) -> Result<()> {
        todo!()
    }

    /// Despawns `key` and the entities it owns, immediately.
    pub fn despawn(&mut self, key: StableKey) -> Result<()> {
        todo!()
    }

    /// Writes the whole component `value` to `key`, adding it if absent, immediately, and
    /// updates the relationship index. Panics for [`InRegion`](crate::InRegion) and
    /// [`PredictedOrigin`](crate::PredictedOrigin), which are the world's.
    pub fn insert<T: Component>(&mut self, key: StableKey, value: T) -> Result<()> {
        todo!()
    }

    /// Removes component `T` from `key`, immediately, returning it; `None` if absent.
    /// Panics for the two components [`insert`](Self::insert) rejects.
    pub fn remove<T: Component>(&mut self, key: StableKey) -> Result<Option<T>> {
        todo!()
    }

    /// Moves `key` and the entities it owns to `region`, immediately.
    pub fn move_to_region(&mut self, key: StableKey, region: RegionId) -> Result<()> {
        todo!()
    }

    /// Allocates the next runtime `NetId` from the server counter.
    pub fn allocate_net_id(&mut self) -> Result<NetId> {
        todo!()
    }

    /// The counter the next runtime `NetId` would be allocated from, for the save.
    #[must_use]
    pub fn next_net_id(&self) -> NonZeroU64 {
        todo!()
    }

    /// Records that the server bound `net` to the predicted entity `predicted`
    /// (architecture section 7). Both keys then name the entity; its origin key stays
    /// `predicted`. The alias table survives [`restore`](Self::restore), and a bind for a
    /// key with no live entity is kept and applies when the entity is spawned, so a bind
    /// received after a snapshot is not lost to the rollback. Fails if `predicted` or `net`
    /// is already bound to something else.
    pub fn bind(&mut self, predicted: PredictionKey, net: NetId) -> Result<()> {
        todo!()
    }

    /// The `NetId` bound to `predicted`, if any.
    #[must_use]
    pub fn alias(&self, predicted: PredictionKey) -> Option<NetId> {
        todo!()
    }

    /// Sets the input tick of `player` for this tick: the tick slot the input being applied
    /// is for, also when the server substitutes a previous input for a lost one. A spawn
    /// commanded from an entity the player has claimed puts it in its `Predicted` key.
    /// Called from an exclusive system at the input sync point, on the server and on the
    /// client alike; the sync point clears every player's input tick first, so a claimed
    /// source whose player has none this tick spawns unpredicted and passes no claim on.
    pub fn set_input_tick(&mut self, player: PlayerId, input_tick: Tick) {
        todo!()
    }

    /// The input tick of `player` this tick, if set.
    #[must_use]
    pub fn input_tick(&self, player: PlayerId) -> Option<Tick> {
        todo!()
    }

    /// The entities residing in `region`, in ascending origin-key order.
    #[must_use]
    pub fn entities_in(&self, region: RegionId) -> Entities<'_> {
        todo!()
    }

    /// The entities whose components refer to `target` through an `EntityRef` field, in
    /// ascending origin-key order: the reverse index of relationships (architecture section
    /// 6), maintained at command apply and by the direct methods here, included in
    /// snapshots.
    #[must_use]
    pub fn referrers(&self, target: StableKey) -> Referrers<'_> {
        todo!()
    }

    /// The entities `owner` owns, in ascending origin-key order.
    #[must_use]
    pub fn owned_by(&self, owner: StableKey) -> Entities<'_> {
        todo!()
    }

    /// A query over every entity matching `Q`. Panics if `Q` names a component twice.
    #[must_use]
    pub fn query<Q: QueryItems>(&mut self) -> Query<'_, Q> {
        todo!()
    }

    /// The RNG stream for `key` in `region` on behalf of `system`, as
    /// [`Ctx::rng`](crate::Ctx::rng) gives a per-region system: keyed by the region seed,
    /// the tick, `system` and `key`. `None` if the region is not loaded. For exclusive
    /// systems and tests.
    #[must_use]
    pub fn rng(&self, region: RegionId, system: SystemKey, key: StableKey) -> Option<Rng> {
        todo!()
    }

    // ---- Resources ----

    /// Inserts a reflected resource, returning the previous value.
    pub fn insert_resource<T: Resource>(&mut self, value: T) -> Option<T> {
        todo!()
    }

    /// A reflected resource.
    #[must_use]
    pub fn resource<T: Resource>(&self) -> Option<&T> {
        todo!()
    }

    /// A reflected resource, for writing.
    #[must_use]
    pub fn resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        todo!()
    }

    /// Removes a reflected resource.
    pub fn remove_resource<T: Resource>(&mut self) -> Option<T> {
        todo!()
    }

    /// Inserts a service resource: a `dyn` trait object from an `-api` crate that `app`
    /// provides (architecture section 4), read by systems as `Res<dyn Trait>` with exactly
    /// this `T`. Not reflected, hashed, saved or snapshotted.
    #[allow(clippy::boxed_local, reason = "facade: the body is todo!()")]
    pub fn insert_service<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        value: Box<T>,
    ) -> Option<Box<T>> {
        todo!()
    }

    /// A service resource.
    #[must_use]
    pub fn service<T: ?Sized + 'static>(&self) -> Option<&T> {
        todo!()
    }

    /// A service resource, for writing.
    #[must_use]
    pub fn service_mut<T: ?Sized + 'static>(&mut self) -> Option<&mut T> {
        todo!()
    }

    // ---- Regions ----

    /// Creates an empty region. Fails if the region exists.
    pub fn create_region(&mut self, config: RegionConfig) -> Result<()> {
        todo!()
    }

    /// Despawns every entity of `region`, drops its resources and forgets it. Export first
    /// to keep its state ([`export_region`](Self::export_region)).
    pub fn unload_region(&mut self, region: RegionId) -> Result<()> {
        todo!()
    }

    /// What is known about `region`.
    #[must_use]
    pub fn region(&self, region: RegionId) -> Option<RegionInfo> {
        todo!()
    }

    /// The loaded regions, in ascending ID order.
    #[must_use]
    pub fn regions(&self) -> Regions<'_> {
        todo!()
    }

    /// Changes a region's mode (architecture section 6).
    pub fn set_region_mode(&mut self, region: RegionId, mode: RegionMode) -> Result<()> {
        todo!()
    }

    /// Inserts a resource of `region`, returning the previous value. Region resources are
    /// hashed with the region, exported with it if persistent, and part of a snapshot.
    pub fn insert_region_resource<T: Resource>(
        &mut self,
        region: RegionId,
        value: T,
    ) -> Result<Option<T>> {
        todo!()
    }

    /// Inserts a resource of `region` from its bytes, validated against the registry.
    pub fn insert_region_resource_erased(
        &mut self,
        region: RegionId,
        value: ComponentData,
    ) -> Result<()> {
        todo!()
    }

    /// A resource of `region`.
    #[must_use]
    pub fn region_resource<T: Resource>(&self, region: RegionId) -> Option<&T> {
        todo!()
    }

    /// A resource of `region`, for writing.
    #[must_use]
    pub fn region_resource_mut<T: Resource>(&mut self, region: RegionId) -> Option<&mut T> {
        todo!()
    }

    /// Every resource of `region` as bytes, in ascending key order, for the world hash and
    /// the export.
    #[must_use]
    pub fn region_resources_erased(&self, region: RegionId) -> ErasedResources<'_> {
        todo!()
    }

    /// Gives `region` its exact-version assets, resolved outside the boundary and applied
    /// at the input sync point (architecture section 14). Returns the previous table.
    pub fn set_region_assets(
        &mut self,
        region: RegionId,
        assets: PinnedTable,
    ) -> Result<Option<PinnedTable>> {
        todo!()
    }

    /// The exact-version assets of `region`, if a table has been set.
    #[must_use]
    pub fn region_assets(&self, region: RegionId) -> Option<&PinnedTable> {
        todo!()
    }

    // ---- Change tracking ----

    /// The rows of `T` changed since `mark`, or `None` if the mark is stale. Panics if `T`
    /// is not a tracked column (replicated or extracted).
    #[must_use]
    pub fn changed_since<T: Component>(&self, mark: ChangeMark) -> Option<ChangedRows<'_, T>> {
        todo!()
    }

    /// Every chunk's column of `T`, for passes that fork over chunks.
    #[must_use]
    pub fn columns<T: Component>(&self) -> Columns<'_, T> {
        todo!()
    }

    /// Every chunk's column of `T`, for bulk writes that record what they changed.
    #[must_use]
    pub fn columns_mut<T: Component>(&mut self) -> ColumnsMut<'_, T> {
        todo!()
    }

    /// Every chunk's column of `component`, as bytes.
    #[must_use]
    pub fn columns_erased(&self, component: ComponentKey) -> ErasedColumns<'_> {
        todo!()
    }

    /// The structural change log.
    #[must_use]
    pub fn structural_log(&self) -> &StructuralLog {
        todo!()
    }

    /// Drops log entries before `cursor`, the minimum over every consumer's cursor.
    pub fn trim_structural_log(&mut self, cursor: LogCursor) {
        todo!()
    }

    // ---- Buffers consumed at sync points ----

    /// Takes the entries of channel `C` written this tick, in command order, keys resolved.
    pub fn take_channel<C: CommandChannel>(&mut self) -> Vec<ChannelEntry<C>> {
        todo!()
    }

    /// Takes the presentation events delivered since the last call, and the cancellations
    /// (architecture section 9.4).
    pub fn take_present_events(&mut self) -> PresentEventBatch {
        todo!()
    }

    // ---- Snapshots ----

    /// Copies the rollback state (architecture section 10). Taken between ticks only.
    #[must_use]
    pub fn snapshot(&self) -> WorldSnapshot {
        todo!()
    }

    /// Replaces the rollback state with `snapshot`, taken from this world. The tick returns
    /// to the snapshot's, change stamps are rebased, the structural log is cleared and a
    /// new epoch starts, so every consumer does a full pass. The alias table and the
    /// presentation-event log are kept. Fails if the snapshot is from another world.
    pub fn restore(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        todo!()
    }

    // ---- Bulk copy ----

    /// Loads `region`, which must be empty: spawns the cooked rows of `baseline` except
    /// those `overlay` destroys, a copy per archetype column plus a remap pass over
    /// reference fields; then applies `overlay`'s records in record order, restores every
    /// predicted origin key and alias from [`PredictedOrigin`](crate::PredictedOrigin), and
    /// inserts the overlay's region resources. A `Destroy` record never touches a live
    /// entity of another region: a cooked entity that moved is a row there, not here. Every
    /// component schema must match the registry's; a mismatch is an error of kind
    /// `VersionMismatch`, and `lactose-persist` migrates the document first. Rows are
    /// stamped at the current tick and logged as spawned; no epoch starts.
    pub fn load_region(
        &mut self,
        region: RegionId,
        baseline: &Baseline,
        overlay: Option<&Overlay>,
    ) -> Result<()> {
        todo!()
    }

    /// Copies the state of `region` out: every persistent component of every entity in
    /// ascending `NetId` order, with the owner beside each row, plus the seed and the
    /// persistent region resources. An entity without a bound `NetId` is an error: the
    /// server binds every entity before a save, and a client never exports.
    pub fn export_region(&self, region: RegionId) -> Result<RegionRows> {
        todo!()
    }
}

/// Iterates entities by origin key.
pub struct Entities<'w> {
    _world: PhantomData<&'w ()>,
}

impl Iterator for Entities<'_> {
    type Item = StableKey;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// Iterates loaded regions.
pub struct Regions<'w> {
    _world: PhantomData<&'w ()>,
}

impl Iterator for Regions<'_> {
    type Item = RegionId;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// One reference to an entity from a component field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Referrer {
    /// The entity holding the reference.
    pub source: StableKey,
    /// The component holding it.
    pub component: ComponentKey,
    /// The field holding it.
    pub field: FieldId,
}

/// Iterates the references to one entity.
pub struct Referrers<'w> {
    _world: PhantomData<&'w ()>,
}

impl Iterator for Referrers<'_> {
    type Item = Referrer;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// Iterates resources as bytes, in ascending key order.
pub struct ErasedResources<'w> {
    _world: PhantomData<&'w ()>,
}

impl<'w> Iterator for ErasedResources<'w> {
    type Item = (ComponentKey, &'w [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
