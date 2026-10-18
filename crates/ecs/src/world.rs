//! The world: entities, components, resources and regions with their change state
//! (architecture section 6), and the direct access that exclusive systems and engine
//! passes use between stages.

use core::any::{Any, TypeId};
use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use lactose_asset_api::PinnedTable;
use lactose_bundle::{Baseline, ComponentData, Overlay};
use lactose_core::{
    ComponentKey, EntityRef, Error, ErrorKind, FieldId, FixedHashMap, NetId, PlayerId,
    PredictionKey, Result, StableKey, SystemKey, Tick,
};
use lactose_reflect::{
    Component, FieldKind, Persistence, Reflect, Registry, Resource, TypeInfo, TypeRole,
};

use crate::bulk::RegionRows;
use crate::change::{
    ChangeMark, ChangedRows, ColumnMut, ColumnRef, Columns, ColumnsMut, ErasedColumn,
    ErasedColumns, LogCursor, Stamp, StructuralChange, StructuralLog, tick_before,
};
use crate::command::{Bundle, ChannelStore, ErasedComponent, Payload, erase_bundle, is_worlds_own};
use crate::event::{EventStore, PresentEventBatch, PresentLog};
use crate::param::{Mut, resource_ref};
use crate::query::{Query, QueryItems};
use crate::region::{
    ClaimedBy, InRegion, OwnedBy, PredictedOrigin, RegionConfig, RegionId, RegionInfo, RegionMode,
};
use crate::rng::{LayoutRng, Rng};
use crate::snapshot::WorldSnapshot;
use crate::storage::{AlignedBytes, Loc, MAX_ALIGN, RowValue, Storage, typed, typed_slice};

/// The first position between ticks: after every position a tick's slots use.
pub(crate) const BETWEEN: u32 = 1 << 31;

/// The positions one slot of a tick spans: its base, then the direct writes made in it.
pub(crate) const SLOT_WIDTH: u32 = 1 << 16;

/// Change stamps are clamped forward, and a new epoch starts, every this many ticks
/// (architecture section 6).
pub(crate) const REBASE_TICKS: u32 = 1 << 30;

/// Which world this is. A system declares the kinds it runs in, and the world applies its
/// own filter to every kernel (architecture section 9.4), so game code never writes one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WorldKind {
    /// The authoritative world. Spawns receive runtime `NetId`s.
    Server,
    /// A client's replicated world: rows hold received state, written at the input sync
    /// point. It runs only the systems that name `client`, which simulation systems do
    /// not (architecture section 5.3).
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
    /// Every world. A system without `worlds` declares `[server, prediction]`, not this.
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

impl LayoutConfig {
    /// `items` reordered as chunk jobs are issued; `salt` varies the shuffle per call site.
    pub(crate) fn order<T>(&self, items: Vec<T>, salt: (u64, u64)) -> Vec<T> {
        match self.chunk_order {
            ChunkOrder::Natural => items,
            ChunkOrder::Reversed => items.into_iter().rev().collect(),
            ChunkOrder::Shuffled => {
                let order = LayoutRng::new(self.seed, salt.0, salt.1).permutation(items.len());
                let mut slots: Vec<Option<T>> = items.into_iter().map(Some).collect();
                order.into_iter().map(|index| slots[index].take().expect("a permutation")).collect()
            }
        }
    }
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
        Self {
            kind,
            registry,
            layout: LayoutConfig::default(),
            tick: Tick(0),
            next_net_id: NonZeroU64::MIN,
        }
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
    /// chunk capacity cannot hold a registered component: a capacity of zero rows, a
    /// component larger than a 16 KiB chunk or aligned to more than eight bytes.
    pub fn build(self) -> Result<World> {
        let registry = &self.registry;
        if !(registry.contains::<InRegion>()
            && registry.contains::<OwnedBy>()
            && registry.contains::<ClaimedBy>()
            && registry.contains::<PredictedOrigin>())
        {
            return Err(Error::new(
                ErrorKind::NotFound,
                "the registry lacks a component the world maintains: register `InRegion`, \
                 `OwnedBy`, `ClaimedBy` and `PredictedOrigin`",
            ));
        }
        if self.layout.chunk_rows == Some(0) {
            return Err(Error::new(ErrorKind::InvalidData, "a chunk holds at least one row"));
        }
        for info in registry.iter().filter(|info| info.role() == TypeRole::Component) {
            let layout = info.layout();
            if layout.size() > lactose_bundle::MAX_COMPONENT_BYTES as usize
                || layout.align() > MAX_ALIGN
            {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "`{}` ({} bytes, aligned to {}) does not fit a chunk: components are at \
                         most 16 KiB and aligned to at most {MAX_ALIGN} bytes",
                        info.name(),
                        layout.size(),
                        layout.align()
                    ),
                ));
            }
        }
        Ok(World {
            identity: Arc::new(()),
            kind: self.kind,
            storage: Storage::with_clock(self.layout.chunk_rows, self.tick),
            registry: self.registry,
            layout: self.layout,
            tick: self.tick,
            epoch: 0,
            point: BETWEEN,
            point_cap: u32::MAX,
            in_run: false,
            next_net_id: self.next_net_id,
            resources: FixedHashMap::new(),
            regions: BTreeMap::new(),
            aliases: Aliases::default(),
            input_ticks: BTreeMap::new(),
            log: StructuralLog::new(0),
            references: References::default(),
            claims: BTreeMap::new(),
            events: EventStore::new(),
            channels: ChannelStore::new(),
            present: PresentLog::default(),
            despawned_predicted: BTreeMap::new(),
            sync_points: 0,
        })
    }
}

/// The prediction-to-network alias table (architecture section 7).
#[derive(Clone, Default)]
pub(crate) struct Aliases {
    pub(crate) by_predicted: BTreeMap<PredictionKey, NetId>,
    pub(crate) by_net: BTreeMap<NetId, PredictionKey>,
}

/// Copies a reflected resource's `Box<T>` into a snapshot.
pub(crate) type CloneFn = fn(&(dyn Any + Send + Sync)) -> Box<dyn Any + Send + Sync>;

/// A world resource: the `Box<T>` of a reflected resource or a service, held as `Any`.
pub(crate) struct ResourceEntry {
    pub(crate) value: Box<dyn Any + Send + Sync>,
    /// How to copy a reflected resource into a snapshot; `None` for a service.
    pub(crate) clone: Option<CloneFn>,
    pub(crate) size: usize,
}

fn clone_resource<T: Resource>(value: &(dyn Any + Send + Sync)) -> Box<dyn Any + Send + Sync> {
    let value: &T = resource_ref(value);
    Box::new(Box::new(*value))
}

/// A loaded region.
#[derive(Clone)]
pub(crate) struct RegionState {
    pub(crate) seed: u64,
    pub(crate) mode: RegionMode,
    pub(crate) resources: BTreeMap<ComponentKey, RegionResource>,
    pub(crate) assets: Option<PinnedTable>,
}

/// A region resource as bytes, aligned for its type.
#[derive(Clone)]
pub(crate) struct RegionResource {
    pub(crate) info: &'static TypeInfo,
    pub(crate) bytes: AlignedBytes,
}

/// The reverse index of relationships: who refers to whom through which field.
#[derive(Clone, Default)]
pub(crate) struct References {
    pub(crate) by_target: BTreeMap<StableKey, BTreeSet<(StableKey, ComponentKey, FieldId)>>,
}

/// Every `EntityRef` in a value of `info`, with the ID of the top-level field holding it.
/// Read from the description, which locates the fields; the bytes have been validated.
pub(crate) fn entity_refs(info: &TypeInfo, bytes: &[u8]) -> Vec<(FieldId, StableKey)> {
    fn walk(kind: &FieldKind, id: FieldId, bytes: &[u8], out: &mut Vec<(FieldId, StableKey)>) {
        match kind {
            FieldKind::EntityRef => {
                let reference: Option<EntityRef> =
                    bytemuck::checked::try_pod_read_unaligned(bytes).ok();
                if let Some(key) = reference.and_then(EntityRef::get) {
                    out.push((id, key));
                }
            }
            FieldKind::Struct(info) => {
                for field in info.fields() {
                    let range = field.offset()..field.offset() + field.size();
                    if let Some(bytes) = bytes.get(range) {
                        walk(&field.kind(), id, bytes, out);
                    }
                }
            }
            FieldKind::Array { element, len, .. } => {
                if let Some(size) = bytes.len().checked_div(*len).filter(|&size| size > 0) {
                    for element_bytes in bytes.chunks_exact(size) {
                        walk(element, id, element_bytes, out);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for field in info.layout().fields() {
        if let Some(id) = field.id() {
            let range = field.offset()..field.offset() + field.size();
            if let Some(bytes) = bytes.get(range) {
                walk(&field.kind(), id, bytes, &mut out);
            }
        }
    }
    out
}

/// The name of the top-level field of a value of `info` that holds a placeholder reference,
/// if one does. The bytes are not validated, since a placeholder is not a valid `EntityRef`:
/// a reference is a placeholder when it equals the encoding `EntityRef::new` gives one, for
/// an index taken from any of its words, so nothing else is ever reported as one.
fn placeholder_field(info: &TypeInfo, bytes: &[u8]) -> Option<&'static str> {
    fn holds(kind: &FieldKind, bytes: &[u8]) -> bool {
        match kind {
            FieldKind::EntityRef => {
                let Ok(words) = bytemuck::try_pod_read_unaligned::<[u32; 4]>(bytes) else {
                    return false;
                };
                words.into_iter().any(|index| {
                    bytes == bytemuck::bytes_of(&EntityRef::new(StableKey::Placeholder(index)))
                })
            }
            FieldKind::Struct(info) => info.fields().iter().any(|field| {
                let range = field.offset()..field.offset() + field.size();
                bytes.get(range).is_some_and(|bytes| holds(&field.kind(), bytes))
            }),
            FieldKind::Array { element, len, .. } => bytes
                .len()
                .checked_div(*len)
                .filter(|&size| size > 0)
                .is_some_and(|size| bytes.chunks_exact(size).any(|bytes| holds(element, bytes))),
            _ => false,
        }
    }
    info.layout().fields().iter().find_map(|field| {
        let range = field.offset()..field.offset() + field.size();
        let bytes = bytes.get(range)?;
        holds(&field.kind(), bytes).then(|| field.name())
    })
}

/// Rewrites every `EntityRef` naming `from` in a value of `info` to name `to`, located by
/// the description; the caller stores bytes it has validated.
fn rewrite_refs(info: &TypeInfo, bytes: &mut [u8], from: StableKey, to: StableKey) {
    fn walk(kind: &FieldKind, bytes: &mut [u8], from: StableKey, to: StableKey) {
        match kind {
            FieldKind::EntityRef => {
                let reference: Option<EntityRef> =
                    bytemuck::checked::try_pod_read_unaligned(bytes).ok();
                if reference.and_then(EntityRef::get) == Some(from) {
                    bytes.copy_from_slice(bytemuck::bytes_of(&EntityRef::new(to)));
                }
            }
            FieldKind::Struct(info) => {
                for field in info.fields() {
                    let range = field.offset()..field.offset() + field.size();
                    if let Some(bytes) = bytes.get_mut(range) {
                        walk(&field.kind(), bytes, from, to);
                    }
                }
            }
            FieldKind::Array { element, len, .. } => {
                if let Some(size) = bytes.len().checked_div(*len).filter(|&size| size > 0) {
                    for element_bytes in bytes.chunks_exact_mut(size) {
                        walk(element, element_bytes, from, to);
                    }
                }
            }
            _ => {}
        }
    }
    for field in info.layout().fields() {
        let range = field.offset()..field.offset() + field.size();
        if let Some(bytes) = bytes.get_mut(range) {
            walk(&field.kind(), bytes, from, to);
        }
    }
}

/// Whether a value of `info` holds an `EntityRef` anywhere.
pub(crate) fn has_entity_ref(info: &TypeInfo) -> bool {
    fn walk(kind: &FieldKind) -> bool {
        match kind {
            FieldKind::EntityRef => true,
            FieldKind::Struct(info) => info.fields().iter().any(|field| walk(&field.kind())),
            FieldKind::Array { element, .. } => walk(element),
            _ => false,
        }
    }
    info.layout().fields().iter().any(|field| walk(&field.kind()))
}

/// Whether `Mut`, `World::get_mut` and `columns_mut` refuse the component: the world
/// maintains it, or it holds a reference whose reverse index is kept at apply.
pub(crate) fn refuses_mut(info: &TypeInfo) -> bool {
    let key = info.key();
    key == InRegion::KEY
        || key == OwnedBy::KEY
        || key == ClaimedBy::KEY
        || key == PredictedOrigin::KEY
        || has_entity_ref(info)
}

/// Whether a column is tracked: replicated or extracted.
pub(crate) fn is_tracked(info: &TypeInfo) -> bool {
    let tracking = info.tracking();
    tracking.replicated || tracking.extracted
}

fn not_found(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::NotFound, message)
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
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
    /// Ties a snapshot to the world it was taken from; compared by address only.
    pub(crate) identity: Arc<()>,
    pub(crate) kind: WorldKind,
    pub(crate) registry: Arc<Registry>,
    pub(crate) layout: LayoutConfig,
    pub(crate) tick: Tick,
    pub(crate) epoch: u32,
    /// The current position in the tick's stamp order.
    pub(crate) point: u32,
    /// The last position a direct write in the current slot may take.
    pub(crate) point_cap: u32,
    /// Whether a schedule run holds the world.
    pub(crate) in_run: bool,
    pub(crate) next_net_id: NonZeroU64,
    pub(crate) storage: Storage,
    pub(crate) resources: FixedHashMap<TypeId, ResourceEntry>,
    pub(crate) regions: BTreeMap<RegionId, RegionState>,
    pub(crate) aliases: Aliases,
    pub(crate) input_ticks: BTreeMap<PlayerId, Tick>,
    pub(crate) log: StructuralLog,
    pub(crate) references: References,
    /// The claim index: every entity with `ClaimedBy { player }`, by player, in ascending
    /// origin-key order. Maintained wherever `ClaimedBy` enters or leaves a row and carried
    /// by a snapshot, so a lookup by player costs that player's claims, never a scan.
    pub(crate) claims: BTreeMap<PlayerId, BTreeSet<StableKey>>,
    pub(crate) events: EventStore,
    pub(crate) channels: ChannelStore,
    pub(crate) present: PresentLog,
    /// Predicted entities despawned, with the tick, so their aliases can be pruned once no
    /// restore can bring them back.
    pub(crate) despawned_predicted: BTreeMap<PredictionKey, Tick>,
    /// Sync points run, salting SOUR's row permutation.
    pub(crate) sync_points: u64,
}

impl World {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// The registry the world was built with.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The layout applied, for a SOUR run to record.
    #[must_use]
    pub fn layout(&self) -> &LayoutConfig {
        &self.layout
    }

    /// The absolute simulation tick: the tick being run, or the last one run between ticks.
    /// It is never rebased; RNG keys and tick-valued fields use it. Stored change stamps
    /// are rebased every 2^30 ticks and at a restore or a whole-world load, which starts a
    /// new epoch for every [`ChangeMark`] and [`LogCursor`].
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Sets the tick between ticks: a client re-syncing to the server's clock, a server
    /// after a load. Starts a new epoch, and moves every change stamp onto the new clock by
    /// its age on the old one, so a mark taken now sees only later writes:
    ///
    /// - Forward (by less than 2^31 ticks), a stamp of the old tick, which a `Changed`
    ///   reader may not have seen yet, moves to the new tick's current point; every older
    ///   stamp moves to before the new tick.
    /// - Back by `k` ticks (a move of 2^31 ticks or more is read as back), a stamp younger
    ///   than `k + 1` ticks, which would lie in the future, moves to the new tick's current
    ///   point; every older stamp keeps its tick.
    ///
    /// A `Changed` reader sees a moved stamp once more at most. Fails during a schedule run.
    pub fn set_tick(&mut self, tick: Tick) -> Result<()> {
        if self.in_run {
            return Err(invalid("the tick is set between ticks, not during a run"));
        }
        let old = self.tick;
        self.tick = tick;
        self.storage.clock = tick;
        self.point = BETWEEN;
        self.point_cap = u32::MAX;
        // Stamps are moved onto the new clock, by their age on the old one. Forward, what
        // was stamped in the old tick may still be pending for a reader and moves to the new
        // point; anything older was seen by every reader and moves to before the new tick.
        // Back by `k` ticks, what is younger than `k` ticks would be in the future and moves
        // to the new point; the rest keeps its tick. A new epoch starts either way.
        let ceiling = Stamp::new(tick, BETWEEN);
        let seen = Stamp::new(Tick(tick.0.wrapping_sub(1)), 0);
        let forward = tick.0.wrapping_sub(old.0);
        let back = old.0.wrapping_sub(tick.0);
        self.clamp_stamps(|stamp| {
            let age = old.0.wrapping_sub(stamp.tick.0);
            if forward != 0 && forward < 1 << 31 {
                if age == 0 { ceiling } else { seen }
            } else if age <= back {
                ceiling
            } else {
                stamp
            }
        });
        self.new_epoch();
        Ok(())
    }

    /// A mark at the current tick and epoch.
    #[must_use]
    pub fn mark(&self) -> ChangeMark {
        ChangeMark::new(self.epoch, Stamp::new(self.tick, self.point))
    }

    /// Whether `mark` is of the current epoch, so change stamps compare against its tick.
    #[must_use]
    pub fn is_current(&self, mark: ChangeMark) -> bool {
        mark.epoch() == self.epoch
    }

    // ---- Entities ----

    /// Whether `key` names a live entity.
    #[must_use]
    pub fn contains(&self, key: StableKey) -> bool {
        self.origin(key).is_some()
    }

    /// The origin key of `key`'s entity: `key` itself, or the `Predicted` key behind a
    /// bound `Net` key. `None` if the entity does not exist.
    #[must_use]
    pub fn origin(&self, key: StableKey) -> Option<StableKey> {
        let live = |key: StableKey| self.storage.entities.contains_key(&key).then_some(key);
        match key {
            StableKey::Placeholder(_) => None,
            StableKey::Predicted(_) => live(key),
            StableKey::Net(net) => live(key).or_else(|| {
                let predicted = self.aliases.by_net.get(&net)?;
                live(StableKey::Predicted(*predicted))
            }),
        }
    }

    /// The region `key` resides in.
    #[must_use]
    pub fn region_of(&self, key: StableKey) -> Option<RegionId> {
        let loc = self.storage.loc(self.origin(key)?)?;
        Some(self.region_at(loc))
    }

    /// Component `T` of `key`, or `None` if the entity does not exist or lacks it.
    #[must_use]
    pub fn get<T: Component>(&self, key: StableKey) -> Option<&T> {
        let loc = self.storage.loc(self.origin(key)?)?;
        self.storage.component(loc, T::KEY).map(typed)
    }

    /// Component `T` of `key` for writing, marking the row changed on a mutable
    /// dereference, like the kernel parameter. Panics for a component the world maintains
    /// or one with an `EntityRef` field; those go through [`insert`](Self::insert).
    #[must_use]
    pub fn get_mut<T: Component>(&mut self, key: StableKey) -> Option<Mut<'_, T>> {
        assert!(
            !refuses_mut(T::SCHEMA.info()),
            "`{}` is maintained by the world or holds a reference: write it with `insert`",
            T::SCHEMA.name()
        );
        let loc = self.storage.loc(self.origin(key)?)?;
        let index = self.storage.archetypes[loc.archetype as usize].column_index(T::KEY)?;
        let now = self.bump();
        let archetype = &mut self.storage.archetypes[loc.archetype as usize];
        let chunk = &mut archetype.chunks[loc.chunk as usize];
        let tick = &mut chunk.column_ticks[index];
        *tick = now;
        let row = chunk.row_view(&archetype.columns, index, loc.row as usize);
        Some(Mut::new(row, now))
    }

    /// Spawns an entity with `key` in `region`, immediately. `key` is a `Net` or
    /// `Predicted` key that no live entity has; a placeholder is an error. Replication and
    /// prediction spawn this way with the server's key; a kernel spawns through
    /// [`Commands::spawn`](crate::Commands::spawn), which assigns the key. A `Predicted`
    /// key gives the entity [`PredictedOrigin`](crate::PredictedOrigin). Storage holds
    /// origin keys: a bound `Net` key inside a reference field of `bundle` is rewritten to
    /// the target's origin key here, as at command apply and [`insert`](Self::insert).
    pub fn spawn<B: Bundle>(&mut self, key: StableKey, region: RegionId, bundle: B) -> Result<()> {
        let components = erase_bundle(bundle);
        if components.iter().any(|component| is_worlds_own(component.key())) {
            return Err(invalid("`InRegion` and `PredictedOrigin` are written by the world only"));
        }
        self.spawn_direct(key, region, components)
    }

    /// The bytes of `component` of `key` as stored (reference fields hold origin keys), or
    /// `None` if the entity does not exist or lacks the component.
    #[must_use]
    pub fn get_erased(&self, key: StableKey, component: ComponentKey) -> Option<&[u8]> {
        let loc = self.storage.loc(self.origin(key)?)?;
        self.storage.component(loc, component)
    }

    /// Writes `data` whole to `key`, immediately, from its bytes: what a consumer that works
    /// from the registry (`lactose-predict`'s corrections, `net`'s upserts) writes instead of
    /// [`insert`](Self::insert). Checked in this order, nothing changing on an error:
    /// `NotFound` if `data.component` is not registered, as `insert` reports it;
    /// `InvalidData` if it is a resource, [`InRegion`](crate::InRegion) or
    /// [`PredictedOrigin`](crate::PredictedOrigin), or `data.bytes` is not the registry's
    /// element size; then the padding is zeroed, a placeholder in a reference field refused
    /// by name (`InvalidData`; `insert` would store the empty reference, but erased bytes
    /// come from the wire or a recorded slot, where a placeholder is never valid) and the
    /// bytes validated through `TypeInfo::validate` (`InvalidData`); `NotFound` if the
    /// entity does not exist. A bound `Net` key in a reference field is stored as the
    /// origin key, as `insert` stores it.
    ///
    /// Adds the component if the row lacks it, an archetype change the structural log
    /// records, and otherwise writes it only if the stored bytes differ. A write stamps the
    /// row as `insert` does, and keeps the reverse index and the claim index current for a
    /// relationship component ([`OwnedBy`](crate::OwnedBy), [`ClaimedBy`](crate::ClaimedBy),
    /// any with an `EntityRef` field); equal bytes change nothing, so no `Changed` reader
    /// sees them. Returns whether it wrote.
    pub fn insert_erased(&mut self, key: StableKey, data: &ComponentData) -> Result<bool> {
        let component = self.erased_value(data)?;
        let origin = self.origin(key).ok_or_else(|| not_found("no such entity"))?;
        if self.get_erased(origin, component.key()) == Some(component.bytes()) {
            return Ok(false);
        }
        let stamp = self.bump();
        self.write_component(origin, component, stamp)?;
        Ok(true)
    }

    /// Removes `component` from `key`, immediately, as [`remove`](Self::remove) does:
    /// `Ok(false)` if the row lacks it; `NotFound` if the component is not registered or the
    /// entity does not exist; `InvalidData` for a resource, [`InRegion`](crate::InRegion)
    /// or [`PredictedOrigin`](crate::PredictedOrigin). The removal is logged, and the
    /// reverse index and the claim index are updated.
    pub fn remove_erased(&mut self, key: StableKey, component: ComponentKey) -> Result<bool> {
        let info = self.component_info(component)?;
        if is_worlds_own(info.key()) {
            return Err(invalid(format!("`{}` is written by the world only", info.name())));
        }
        let origin = self.origin(key).ok_or_else(|| not_found("no such entity"))?;
        Ok(self.remove_component(origin, component).is_some())
    }

    /// Spawns `key` in `region` with `components`, immediately, from their bytes: what
    /// [`spawn`](Self::spawn) does for a consumer that works from the registry. Every entry
    /// is checked as [`insert_erased`](Self::insert_erased) checks it, and two entries for
    /// one component are `InvalidData`; the key rules are `spawn`'s (a `Net` or `Predicted`
    /// key that no live entity has, `InvalidData` otherwise or for a placeholder; a
    /// `Predicted` key gives [`PredictedOrigin`](crate::PredictedOrigin)), and `NotFound` if
    /// the region is not loaded. Everything is checked before anything changes, so a
    /// refused spawn changes nothing.
    pub fn spawn_erased(
        &mut self,
        key: StableKey,
        region: RegionId,
        components: &[ComponentData],
    ) -> Result<()> {
        let components =
            components.iter().map(|data| self.erased_value(data)).collect::<Result<Vec<_>>>()?;
        self.spawn_direct(key, region, components)
    }

    /// Despawns `key` and the entities it owns, immediately.
    pub fn despawn(&mut self, key: StableKey) -> Result<()> {
        let origin = self.origin(key).ok_or_else(|| not_found("no such entity"))?;
        self.despawn_tree(origin);
        Ok(())
    }

    /// Writes the whole component `value` to `key`, adding it if absent, immediately, and
    /// updates the relationship index. Panics for [`InRegion`](crate::InRegion) and
    /// [`PredictedOrigin`](crate::PredictedOrigin), which are the world's. `NotFound` for a
    /// missing entity or an unregistered component; a refused insert changes nothing.
    pub fn insert<T: Component>(&mut self, key: StableKey, value: T) -> Result<()> {
        assert!(!is_worlds_own(T::KEY), "`{}` is written by the world only", T::SCHEMA.name());
        let origin = self.origin(key).ok_or_else(|| not_found("no such entity"))?;
        let mut component = ErasedComponent::of(value);
        self.resolve_references(&mut component, None, None);
        self.checked_info(&component)?;
        let stamp = self.bump();
        self.write_component(origin, component, stamp)
    }

    /// Removes component `T` from `key`, immediately, returning it; `None` if absent.
    /// Panics for the two components [`insert`](Self::insert) rejects.
    pub fn remove<T: Component>(&mut self, key: StableKey) -> Result<Option<T>> {
        assert!(!is_worlds_own(T::KEY), "`{}` is written by the world only", T::SCHEMA.name());
        let origin = self.origin(key).ok_or_else(|| not_found("no such entity"))?;
        Ok(self
            .remove_component(origin, T::KEY)
            .map(|bytes| bytemuck::checked::pod_read_unaligned(&bytes)))
    }

    /// Moves `key` and the entities it owns to `region`, immediately.
    pub fn move_to_region(&mut self, key: StableKey, region: RegionId) -> Result<()> {
        let origin = self.origin(key).ok_or_else(|| not_found("no such entity"))?;
        if !self.regions.contains_key(&region) {
            return Err(not_found("the region is not loaded"));
        }
        let stamp = self.bump();
        self.move_tree(origin, region, stamp);
        Ok(())
    }

    /// Allocates the next runtime `NetId` from the server counter, passing over any that
    /// already names a live entity or a bind, such as one spawned directly with its key.
    pub fn allocate_net_id(&mut self) -> Result<NetId> {
        loop {
            let id = NetId::runtime(self.next_net_id).ok_or_else(|| {
                Error::new(ErrorKind::ResourceExhausted, "the runtime NetId counter is exhausted")
            })?;
            self.next_net_id = self.next_net_id.saturating_add(1);
            let used = self.storage.entities.contains_key(&StableKey::Net(id))
                || self.aliases.by_net.contains_key(&id);
            if !used {
                return Ok(id);
            }
        }
    }

    /// The counter the next runtime `NetId` would be allocated from, for the save.
    #[must_use]
    pub fn next_net_id(&self) -> NonZeroU64 {
        self.next_net_id
    }

    /// Records that the server bound `net` to the predicted entity `predicted`
    /// (architecture section 7). Both keys then name the entity; its origin key stays
    /// `predicted`. The alias table survives [`restore`](Self::restore), and a bind for a
    /// key with no live entity is kept and applies when the entity is spawned, so a bind
    /// received after a snapshot is not lost to the rollback. A reference stored as `net`
    /// before the bind, such as an owner loaded from another region's save, is rewritten
    /// to `predicted`, in rows and region resources alike, so stored bytes are origin keys
    /// whatever the order of loads and inserts. Fails if
    /// `predicted` or `net` is already bound to something else, or if `net` names a live
    /// entity of its own, whose references a bind would redirect.
    pub fn bind(&mut self, predicted: PredictionKey, net: NetId) -> Result<()> {
        let bound_net = self.aliases.by_predicted.get(&predicted);
        let bound_predicted = self.aliases.by_net.get(&net);
        if bound_net.is_some_and(|&bound| bound != net)
            || bound_predicted.is_some_and(|&bound| bound != predicted)
        {
            return Err(invalid("one of the two keys is already bound to another"));
        }
        if self.storage.entities.contains_key(&StableKey::Net(net)) {
            return Err(invalid("the NetId names a live entity of its own"));
        }
        self.aliases.by_predicted.insert(predicted, net);
        self.aliases.by_net.insert(net, predicted);
        self.rekey_references(StableKey::Net(net), StableKey::Predicted(predicted));
        Ok(())
    }

    /// The `NetId` bound to `predicted`, if any.
    #[must_use]
    pub fn alias(&self, predicted: PredictionKey) -> Option<NetId> {
        self.aliases.by_predicted.get(&predicted).copied()
    }

    /// Sets the input tick of `player` for this tick: the tick slot the input being applied
    /// is for, also when the server substitutes a previous input for a lost one. A spawn
    /// commanded from an entity the player has claimed puts it in its `Predicted` key.
    /// Called from an exclusive system at the input sync point, on the server and on the
    /// client alike; the sync point clears every player's input tick first, so a claimed
    /// source whose player has none this tick spawns unpredicted and passes no claim on.
    pub fn set_input_tick(&mut self, player: PlayerId, input_tick: Tick) {
        self.input_ticks.insert(player, input_tick);
    }

    /// The input tick of `player` this tick, if set.
    #[must_use]
    pub fn input_tick(&self, player: PlayerId) -> Option<Tick> {
        self.input_ticks.get(&player).copied()
    }

    /// The entities residing in `region`, in ascending origin-key order.
    #[must_use]
    pub fn entities_in(&self, region: RegionId) -> Entities<'_> {
        let keys: Vec<StableKey> = self
            .storage
            .entities
            .iter()
            .filter(|&(_, &loc)| self.region_at(loc) == region)
            .map(|(&key, _)| key)
            .collect();
        Entities { keys: keys.into_iter(), _world: PhantomData }
    }

    /// The entities whose components refer to `target` through an `EntityRef` field, in
    /// ascending origin-key order: the reverse index of relationships (architecture section
    /// 6), maintained at command apply and by the direct methods here, included in
    /// snapshots.
    #[must_use]
    pub fn referrers(&self, target: StableKey) -> Referrers<'_> {
        let target = self.canonical(target);
        let referrers: Vec<Referrer> = self
            .references
            .by_target
            .get(&target)
            .into_iter()
            .flatten()
            .map(|&(source, component, field)| Referrer { source, component, field })
            .collect();
        Referrers { referrers: referrers.into_iter(), _world: PhantomData }
    }

    /// The entities `owner` owns, in ascending origin-key order.
    #[must_use]
    pub fn owned_by(&self, owner: StableKey) -> Entities<'_> {
        Entities { keys: self.owned(self.canonical(owner)).into_iter(), _world: PhantomData }
    }

    /// The entities with [`ClaimedBy`](crate::ClaimedBy) `{ player }`, in ascending
    /// origin-key order, from an index the world maintains wherever `ClaimedBy` enters or
    /// leaves a row (a spawn, `insert`, `remove`, a despawn, a command, a region load or
    /// unload) and carries in a snapshot, so the cost is the player's claims, never a scan.
    /// Allocates nothing. How an input or session applier finds the player's entity.
    #[must_use]
    pub fn claimed(&self, player: PlayerId) -> Claimed<'_> {
        Claimed { keys: self.claims.get(&player).map(BTreeSet::iter) }
    }

    /// A query over every entity matching `Q`. Panics if `Q` breaks the alias rule of
    /// [`QueryItems`], or names a `Mut` on a component [`get_mut`](Self::get_mut) rejects.
    #[must_use]
    pub fn query<Q: QueryItems>(&mut self) -> Query<'_, Q> {
        Query::new(self)
    }

    /// The RNG stream for `key` in `region` on behalf of `system`, as
    /// [`Ctx::rng`](crate::Ctx::rng) gives a per-region system: keyed by the region seed,
    /// the tick, `system` and `key`. `None` if the region is not loaded. For exclusive
    /// systems and tests.
    #[must_use]
    pub fn rng(&self, region: RegionId, system: SystemKey, key: StableKey) -> Option<Rng> {
        let state = self.regions.get(&region)?;
        Some(Rng::keyed(state.seed, self.tick, system, None, key))
    }

    // ---- Resources ----

    /// Inserts a reflected resource, returning the previous value.
    pub fn insert_resource<T: Resource>(&mut self, value: T) -> Option<T> {
        let entry = ResourceEntry {
            value: Box::new(Box::new(value)),
            clone: Some(clone_resource::<T>),
            size: size_of::<T>(),
        };
        let previous = self.resources.insert(TypeId::of::<T>(), entry)?;
        previous.value.downcast::<Box<T>>().ok().map(|value| **value)
    }

    /// A reflected resource.
    #[must_use]
    pub fn resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get(&TypeId::of::<T>()).map(|entry| resource_ref(&*entry.value))
    }

    /// A reflected resource, for writing.
    #[must_use]
    pub fn resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        let entry = self.resources.get_mut(&TypeId::of::<T>())?;
        let value: &mut Box<T> = entry.value.downcast_mut().expect("resources hold `Box<T>`");
        Some(value)
    }

    /// Removes a reflected resource.
    pub fn remove_resource<T: Resource>(&mut self) -> Option<T> {
        let entry = self.resources.remove(&TypeId::of::<T>())?;
        entry.value.downcast::<Box<T>>().ok().map(|value| **value)
    }

    /// Inserts a service resource: a `dyn` trait object from an `-api` crate that `app`
    /// provides (architecture section 4), read by systems as `Res<dyn Trait>` with exactly
    /// this `T`. Not reflected, hashed, saved or snapshotted.
    pub fn insert_service<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        value: Box<T>,
    ) -> Option<Box<T>> {
        let entry = ResourceEntry { value: Box::new(value), clone: None, size: 0 };
        let previous = self.resources.insert(TypeId::of::<T>(), entry)?;
        previous.value.downcast::<Box<T>>().ok().map(|value| *value)
    }

    /// A service resource.
    #[must_use]
    pub fn service<T: ?Sized + 'static>(&self) -> Option<&T> {
        self.resources.get(&TypeId::of::<T>()).map(|entry| resource_ref(&*entry.value))
    }

    /// A service resource, for writing.
    #[must_use]
    pub fn service_mut<T: ?Sized + 'static>(&mut self) -> Option<&mut T> {
        let entry = self.resources.get_mut(&TypeId::of::<T>())?;
        let value: &mut Box<T> = entry.value.downcast_mut().expect("resources hold `Box<T>`");
        Some(&mut **value)
    }

    // ---- Regions ----

    /// Creates an empty region. Fails if the region exists.
    pub fn create_region(&mut self, config: RegionConfig) -> Result<()> {
        if self.regions.contains_key(&config.id) {
            return Err(invalid("the region exists"));
        }
        self.regions.insert(
            config.id,
            RegionState {
                seed: config.seed,
                mode: config.mode,
                resources: BTreeMap::new(),
                assets: None,
            },
        );
        Ok(())
    }

    /// Despawns every entity of `region`, drops its resources and forgets it. Export first
    /// to keep its state ([`export_region`](Self::export_region)).
    pub fn unload_region(&mut self, region: RegionId) -> Result<()> {
        if !self.regions.contains_key(&region) {
            return Err(not_found("the region is not loaded"));
        }
        for key in self.entities_in(region).collect::<Vec<_>>() {
            self.despawn_one(key);
            // An unloaded entity is not gone: its alias stays for the export of what it
            // owns elsewhere and for the region's reload.
            if let StableKey::Predicted(predicted) = key {
                self.despawned_predicted.remove(&predicted);
            }
        }
        self.regions.remove(&region);
        Ok(())
    }

    /// What is known about `region`.
    #[must_use]
    pub fn region(&self, region: RegionId) -> Option<RegionInfo> {
        let state = self.regions.get(&region)?;
        let entity_count =
            self.storage.entities.values().filter(|&&loc| self.region_at(loc) == region).count();
        Some(RegionInfo {
            id: region,
            seed: state.seed,
            mode: state.mode,
            entity_count: entity_count as u32,
        })
    }

    /// The mode of `region`, or `None` if it is not loaded. Unlike
    /// [`region`](Self::region), which counts the region's entities, it costs a lookup: what
    /// a consumer that reads every region's mode at every sync point calls.
    #[must_use]
    pub fn region_mode(&self, region: RegionId) -> Option<RegionMode> {
        self.regions.get(&region).map(|state| state.mode)
    }

    /// The loaded regions, in ascending ID order.
    #[must_use]
    pub fn regions(&self) -> Regions<'_> {
        let ids: Vec<RegionId> = self.regions.keys().copied().collect();
        Regions { ids: ids.into_iter(), _world: PhantomData }
    }

    /// Changes a region's mode (architecture section 6).
    pub fn set_region_mode(&mut self, region: RegionId, mode: RegionMode) -> Result<()> {
        let state = self.regions.get_mut(&region).ok_or_else(|| not_found("no such region"))?;
        state.mode = mode;
        Ok(())
    }

    /// Inserts a resource of `region`, returning the previous value. Region resources are
    /// hashed with the region, exported with it if persistent, and part of a snapshot.
    /// Fails if the region is not loaded, `T` is not registered or the value is not valid
    /// to store (a placeholder in a reference field). A bound `Net` key in a reference
    /// field is stored as the origin key; references in resources are not in the reverse
    /// index and may dangle.
    pub fn insert_region_resource<T: Resource>(
        &mut self,
        region: RegionId,
        value: T,
    ) -> Result<Option<T>> {
        let info = self
            .registry
            .get_by_type(TypeId::of::<T>())
            .ok_or_else(|| not_found(format!("`{}` is not registered", T::SCHEMA.name())))?;
        let bytes = self.canonical_resource(info, bytemuck::bytes_of(&value))?;
        let state = self.regions.get_mut(&region).ok_or_else(|| not_found("no such region"))?;
        let resource = RegionResource { info, bytes };
        Ok(state.resources.insert(T::KEY, resource).and_then(|previous| {
            bytemuck::checked::try_pod_read_unaligned(previous.bytes.as_bytes()).ok()
        }))
    }

    /// Inserts a resource of `region` from its bytes, validated against the registry.
    pub fn insert_region_resource_erased(
        &mut self,
        region: RegionId,
        value: ComponentData,
    ) -> Result<()> {
        let info = self
            .registry
            .get(value.component)
            .ok_or_else(|| not_found("the resource is not registered"))?;
        if info.role() != TypeRole::Resource {
            return Err(invalid(format!("`{}` is not a resource", info.name())));
        }
        let bytes = self.canonical_resource(info, &value.bytes)?;
        let state = self.regions.get_mut(&region).ok_or_else(|| not_found("no such region"))?;
        state.resources.insert(value.component, RegionResource { info, bytes });
        Ok(())
    }

    /// A resource of `region`. Region resources are written through
    /// [`insert_region_resource`](Self::insert_region_resource) and
    /// [`update_region_resource`](Self::update_region_resource), which store only valid
    /// values.
    #[must_use]
    pub fn region_resource<T: Resource>(&self, region: RegionId) -> Option<&T> {
        let resource = self.regions.get(&region)?.resources.get(&T::KEY)?;
        bytemuck::checked::try_from_bytes(resource.bytes.as_bytes()).ok()
    }

    /// Updates the resource `T` of `region` through `f`, which receives a copy of it. The
    /// result is validated and a bound `Net` key in its reference fields is stored as the
    /// origin key, as [`insert_region_resource`](Self::insert_region_resource) does; it
    /// replaces the stored value only if it is valid to store. Fails with `NotFound` if the
    /// region is not loaded or lacks `T`, and with `InvalidData`, keeping the old value, if
    /// the result is not valid (a placeholder in a reference field).
    pub fn update_region_resource<T: Resource>(
        &mut self,
        region: RegionId,
        f: impl FnOnce(&mut T),
    ) -> Result<()> {
        let state = self.regions.get(&region).ok_or_else(|| not_found("no such region"))?;
        let resource = state
            .resources
            .get(&T::KEY)
            .ok_or_else(|| not_found(format!("the region has no `{}`", T::SCHEMA.name())))?;
        let info = resource.info;
        let mut value: T = bytemuck::checked::try_pod_read_unaligned(resource.bytes.as_bytes())
            .map_err(|_| invalid("the stored resource is not a valid value"))?;
        f(&mut value);
        let bytes = self.canonical_resource(info, bytemuck::bytes_of(&value))?;
        let state = self.regions.get_mut(&region).expect("checked above");
        state.resources.get_mut(&T::KEY).expect("checked above").bytes = bytes;
        Ok(())
    }

    /// Inserts the transient resource `resource` of `region`, zeroed, as the input sync point
    /// leaves it: how a load restores the presence of a transient region resource, which an
    /// export lists in [`RegionRows::transient_resources`](crate::RegionRows::transient_resources)
    /// and never saves. Fails if the region is not loaded, or `resource` is not a registered
    /// transient resource.
    pub fn insert_region_resource_zeroed(
        &mut self,
        region: RegionId,
        resource: ComponentKey,
    ) -> Result<()> {
        let info = self
            .registry
            .get(resource)
            .ok_or_else(|| not_found("the resource is not registered"))?;
        if info.role() != TypeRole::Resource || info.persistence() != Persistence::Transient {
            return Err(invalid(format!("`{}` is not a transient resource", info.name())));
        }
        let state = self.regions.get_mut(&region).ok_or_else(|| not_found("no such region"))?;
        // Registration checked that the all-zero value of a transient type is valid.
        let bytes = AlignedBytes::zeroed(info.layout().size());
        state.resources.insert(resource, RegionResource { info, bytes });
        Ok(())
    }

    /// Every resource of `region` as bytes, in ascending key order, for the world hash and
    /// the export.
    #[must_use]
    pub fn region_resources_erased(&self, region: RegionId) -> ErasedResources<'_> {
        let resources: Vec<(ComponentKey, &[u8])> = self
            .regions
            .get(&region)
            .into_iter()
            .flat_map(|state| state.resources.iter())
            .map(|(&key, resource)| (key, resource.bytes.as_bytes()))
            .collect();
        ErasedResources { resources: resources.into_iter() }
    }

    /// Gives `region` its exact-version assets, resolved outside the boundary and applied
    /// at the input sync point (architecture section 14). Returns the previous table.
    pub fn set_region_assets(
        &mut self,
        region: RegionId,
        assets: PinnedTable,
    ) -> Result<Option<PinnedTable>> {
        let state = self.regions.get_mut(&region).ok_or_else(|| not_found("no such region"))?;
        Ok(state.assets.replace(assets))
    }

    /// The exact-version assets of `region`, if a table has been set.
    #[must_use]
    pub fn region_assets(&self, region: RegionId) -> Option<&PinnedTable> {
        self.regions.get(&region)?.assets.as_ref()
    }

    // ---- Change tracking ----

    /// The rows of `T` changed since `mark`, or `None` if the mark is stale. Panics if `T`
    /// is not a tracked column (replicated or extracted).
    #[must_use]
    pub fn changed_since<T: Component>(&self, mark: ChangeMark) -> Option<ChangedRows<'_, T>> {
        assert!(
            is_tracked(T::SCHEMA.info()),
            "`{}` is not tracked: only replicated or extracted columns carry row stamps",
            T::SCHEMA.name()
        );
        if !self.is_current(mark) {
            return None;
        }
        let since = mark.stamp();
        let mut rows = Vec::new();
        for (archetype, index) in self.archetypes_with(T::KEY) {
            let columns = &self.storage.archetypes[archetype].columns;
            for chunk in &self.storage.archetypes[archetype].chunks {
                let stamps = chunk.stamps[index].as_ref().expect("a tracked column");
                let values = typed_slice::<T>(chunk.column(&columns[index]), chunk.len());
                for (row, value) in values.iter().enumerate() {
                    if stamps.get(row).after(since, self.tick) {
                        rows.push((chunk.keys[row], value));
                    }
                }
            }
        }
        Some(ChangedRows { rows: rows.into_iter() })
    }

    /// Every chunk's column of `T`, for passes that fork over chunks.
    #[must_use]
    pub fn columns<T: Component>(&self) -> Columns<'_, T> {
        let mut columns = Vec::new();
        for (archetype, index) in self.archetypes_with(T::KEY) {
            let arch = &self.storage.archetypes[archetype];
            let region_index = arch.column_index(InRegion::KEY).expect("every row has a region");
            for chunk in &arch.chunks {
                columns.push(ColumnRef {
                    keys: &chunk.keys,
                    regions: typed_slice(chunk.column(&arch.columns[region_index]), chunk.len()),
                    values: typed_slice(chunk.column(&arch.columns[index]), chunk.len()),
                    row_ticks: chunk.stamps[index].as_ref().map(|s| &s.ticks[..chunk.len()]),
                    column_tick: chunk.column_ticks[index].tick,
                });
            }
        }
        let columns = self.layout.order(columns, (self.tick.0.into(), T::KEY.0));
        Columns { columns: columns.into_iter() }
    }

    /// Every chunk's column of `T`, for bulk writes that record what they changed. Panics
    /// for a component [`get_mut`](Self::get_mut) rejects.
    #[must_use]
    pub fn columns_mut<T: Component>(&mut self) -> ColumnsMut<'_, T> {
        assert!(
            !refuses_mut(T::SCHEMA.info()),
            "`{}` is maintained by the world or holds a reference: write it with `insert`",
            T::SCHEMA.name()
        );
        let now = self.bump();
        let order_salt = (self.tick.0.into(), T::KEY.0);
        let mut columns = Vec::new();
        for arch in &mut self.storage.archetypes {
            let (Some(index), Some(region_index)) =
                (arch.column_index(T::KEY), arch.column_index(InRegion::KEY))
            else {
                continue;
            };
            for chunk in &mut arch.chunks {
                let len = chunk.len();
                let views = chunk.split(&arch.columns);
                let keys = views.keys;
                let mut views: Vec<Option<_>> = views.columns.into_iter().map(Some).collect();
                let region = views[region_index].take().expect("one view per column");
                let regions = typed_slice::<RegionId>(region.bytes, len).to_vec();
                let view = views[index].take().expect("one view per column");
                columns.push(ColumnMut {
                    keys,
                    regions,
                    values: crate::storage::typed_slice_mut(view.bytes, len),
                    stamps: view.stamps,
                    column_tick: view.column_tick,
                    now,
                });
            }
        }
        let columns = self.layout.order(columns, order_salt);
        ColumnsMut { columns: columns.into_iter() }
    }

    /// Every chunk's column of `component`, as bytes.
    #[must_use]
    pub fn columns_erased(&self, component: ComponentKey) -> ErasedColumns<'_> {
        let mut columns = Vec::new();
        for (archetype, index) in self.archetypes_with(component) {
            let arch = &self.storage.archetypes[archetype];
            let region_index = arch.column_index(InRegion::KEY).expect("every row has a region");
            for chunk in &arch.chunks {
                columns.push(ErasedColumn {
                    component,
                    element_size: arch.columns[index].size,
                    keys: &chunk.keys,
                    regions: typed_slice(chunk.column(&arch.columns[region_index]), chunk.len()),
                    bytes: chunk.column(&arch.columns[index]),
                    stamps: chunk.stamps[index].as_ref(),
                    column_tick: chunk.column_ticks[index].tick,
                    epoch: self.epoch,
                    now: self.tick,
                });
            }
        }
        let columns = self.layout.order(columns, (self.tick.0.into(), component.0));
        ErasedColumns { columns: columns.into_iter() }
    }

    /// The structural change log.
    #[must_use]
    pub fn structural_log(&self) -> &StructuralLog {
        &self.log
    }

    /// Drops log entries before `cursor`, the minimum over every consumer's cursor.
    pub fn trim_structural_log(&mut self, cursor: LogCursor) {
        self.log.trim(cursor);
    }

    // ---- Buffers consumed at sync points ----

    /// Takes the presentation events delivered since the last call, and the cancellations
    /// (architecture section 9.4).
    pub fn take_present_events(&mut self) -> PresentEventBatch {
        self.present.take(self.tick)
    }

    // ---- Snapshots ----

    /// Copies the rollback state (architecture section 10). Taken between ticks only.
    #[must_use]
    pub fn snapshot(&self) -> WorldSnapshot {
        debug_assert!(!self.in_run, "a snapshot is taken between ticks");
        WorldSnapshot::take(self)
    }

    /// Replaces the rollback state with `snapshot`, taken from this world: the tick returns
    /// to the snapshot's, every column comes back with its change stamps exactly as
    /// snapshotted (so a re-simulated tick's `Changed` filters see what the first run saw),
    /// the set of loaded regions is the snapshot's, the structural log is cleared and a
    /// new epoch starts, so every consumer does a full pass. The alias table and the
    /// delivered presentation-event keys are kept; emissions after the snapshot that were
    /// never taken are dropped. A reference the snapshot holds under a `Net` key bound since
    /// goes back to the origin key, as the bind left it, unless the restore revives a `Net`
    /// entity of that key, despawned after the snapshot and bound before the restore: its
    /// references stay on it until `lactose-predict` despawns it again, which moves them to
    /// the origin key. A predicted entity
    /// live before and absent after is recorded as despawned at the snapshot's tick, for
    /// [`set_rollback_horizon`](Self::set_rollback_horizon). Fails if the snapshot is from
    /// another world, or during a schedule run.
    pub fn restore(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        if self.in_run {
            return Err(invalid("a restore happens between ticks, not during a run"));
        }
        snapshot.restore_into(self)
    }

    /// Tells the world the oldest tick a restore can return to, so it can prune the
    /// delivered presentation-event keys below it and the aliases of entities despawned
    /// before it. `lactose-predict` calls it with the ring's oldest tick; the server, which
    /// never rolls back, with the current tick.
    pub fn set_rollback_horizon(&mut self, oldest: Tick) {
        self.present.prune(oldest);
        let gone: Vec<PredictionKey> = self
            .despawned_predicted
            .iter()
            .filter(|&(_, &tick)| tick_before(tick, oldest))
            .map(|(&key, _)| key)
            .collect();
        for key in gone {
            self.despawned_predicted.remove(&key);
            if !self.storage.entities.contains_key(&StableKey::Predicted(key))
                && let Some(net) = self.aliases.by_predicted.remove(&key)
            {
                self.aliases.by_net.remove(&net);
            }
        }
    }

    // ---- Bulk copy ----

    /// Loads rows into `region`, which exists ([`create_region`](Self::create_region) with
    /// the saved seed) and is empty: spawns the cooked rows of `baseline` except those
    /// `overlay` destroys, a copy per archetype column plus a remap pass over reference
    /// fields; then applies `overlay`'s records in record order and restores every
    /// predicted origin key and alias from [`PredictedOrigin`](crate::PredictedOrigin).
    /// Reference fields keep the origin keys they were exported with. A `Destroy` record
    /// never touches a live entity of another region: a cooked entity that moved is a row
    /// there, not here. Region resources are not in an overlay; the caller inserts them
    /// with [`insert_region_resource_erased`](Self::insert_region_resource_erased) from the
    /// save or the relevance message, and the transient ones the export listed with
    /// [`insert_region_resource_zeroed`](Self::insert_region_resource_zeroed). Every
    /// component schema must match the registry's; a mismatch is an error of kind
    /// `VersionMismatch`, and `lactose-persist` migrates the document first. Rows are
    /// stamped at the current point and logged as spawned; no epoch starts.
    pub fn load_region(
        &mut self,
        region: RegionId,
        baseline: &Baseline,
        overlay: Option<&Overlay>,
    ) -> Result<()> {
        crate::bulk::load_region(self, region, baseline, overlay)
    }

    /// Copies the state of `region` out: every persistent component of every entity in
    /// ascending `NetId` order, reference fields as stored, with the owner beside each row,
    /// plus the seed and the persistent region resources. An entity without a bound `NetId`
    /// is an error: the server binds every entity before a save, and a client never
    /// exports.
    pub fn export_region(&self, region: RegionId) -> Result<RegionRows> {
        crate::bulk::export_region(self, region)
    }
}

// The world's own operations, shared by the direct methods, command apply and bulk copy.
impl World {
    /// The next direct-write stamp: a position after every one used so far in this slot.
    pub(crate) fn bump(&mut self) -> Stamp {
        self.point = self.point.saturating_add(1).min(self.point_cap);
        Stamp::new(self.tick, self.point)
    }

    /// Starts a new epoch: every mark and cursor goes stale, and the log is emptied.
    pub(crate) fn new_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.log.reset(self.epoch);
    }

    /// Advances the tick for a run, clamping stamps forward at every 2^30 ticks.
    pub(crate) fn advance_tick(&mut self) {
        self.tick = Tick(self.tick.0.wrapping_add(1));
        self.storage.clock = self.tick;
        self.point = 0;
        self.point_cap = 0;
        // Every 2^30 ticks, stamps older than 2^30 ticks come forward to that age, so every
        // stored stamp stays less than 2^31 ticks older than now and orders correctly
        // across the wrap.
        if self.tick.0.is_multiple_of(REBASE_TICKS) {
            let now = self.tick;
            let floor = Stamp::new(Tick(now.0.wrapping_sub(REBASE_TICKS)), 0);
            self.clamp_stamps(|stamp| stamp.later(floor, now));
            self.new_epoch();
        }
    }

    /// Maps every row stamp and column tick through `clamp`.
    fn clamp_stamps(&mut self, clamp: impl Fn(Stamp) -> Stamp) {
        for arch in &mut self.storage.archetypes {
            for chunk in &mut arch.chunks {
                for tick in &mut chunk.column_ticks {
                    *tick = clamp(*tick);
                }
                for stamps in chunk.stamps.iter_mut().flatten() {
                    for (tick, position) in stamps.ticks.iter_mut().zip(&mut stamps.positions) {
                        let stamp = clamp(Stamp::new(*tick, *position));
                        (*tick, *position) = (stamp.tick, stamp.position);
                    }
                }
            }
        }
    }

    /// The key that names `key`'s entity in storage, whether or not it is live: the
    /// `Predicted` key behind a bound `Net` key whose own entity does not exist.
    pub(crate) fn canonical(&self, key: StableKey) -> StableKey {
        if let StableKey::Net(net) = key
            && !self.storage.entities.contains_key(&key)
            && let Some(&predicted) = self.aliases.by_net.get(&net)
        {
            return StableKey::Predicted(predicted);
        }
        key
    }

    /// The `Net` key that also names the predicted entity `origin`, if bound.
    pub(crate) fn net_alias(&self, origin: StableKey) -> Option<StableKey> {
        match origin {
            StableKey::Predicted(predicted) => {
                self.aliases.by_predicted.get(&predicted).map(|&net| StableKey::Net(net))
            }
            _ => None,
        }
    }

    pub(crate) fn region_at(&self, loc: Loc) -> RegionId {
        let bytes = self.storage.component(loc, InRegion::KEY).expect("every row has a region");
        typed::<InRegion>(bytes).region
    }

    /// Every archetype that has `component`, with the column's index.
    pub(crate) fn archetypes_with(&self, component: ComponentKey) -> Vec<(usize, usize)> {
        self.storage
            .archetypes
            .iter()
            .enumerate()
            .filter_map(|(archetype, arch)| Some((archetype, arch.column_index(component)?)))
            .collect()
    }

    /// The live entities `owner` owns, ascending.
    pub(crate) fn owned(&self, owner: StableKey) -> Vec<StableKey> {
        let mut owned: Vec<StableKey> = self
            .references
            .by_target
            .get(&owner)
            .into_iter()
            .flatten()
            .filter(|&&(source, component, _)| {
                component == OwnedBy::KEY && self.storage.entities.contains_key(&source)
            })
            .map(|&(source, _, _)| source)
            .collect();
        owned.dedup();
        owned
    }

    /// `root` and every entity it owns, directly or through owned entities, each once:
    /// `root` first, then breadth first in ascending key order.
    fn ownership_tree(&self, root: StableKey) -> Vec<StableKey> {
        let mut seen = BTreeSet::from([root]);
        let mut order = vec![root];
        let mut queue = VecDeque::from([root]);
        while let Some(owner) = queue.pop_front() {
            for owned in self.owned(owner) {
                if seen.insert(owned) {
                    order.push(owned);
                    queue.push_back(owned);
                }
            }
        }
        order
    }

    /// Rewrites the references of `component` to origin keys: a bound `Net` key to the
    /// predicted origin. At command apply (`placeholders` given) a placeholder resolves
    /// through the buffer's spawns and a reference to an entity that does not exist, other
    /// than `spawning`, is written empty (architecture section 6, "validated at apply"); a
    /// direct write keeps it, as replication and a load do, since the target may arrive
    /// later, and a placeholder there names nothing and is written empty.
    pub(crate) fn resolve_references(
        &self,
        component: &mut ErasedComponent,
        placeholders: Option<&[Option<StableKey>]>,
        spawning: Option<StableKey>,
    ) {
        let Payload::Typed(value) = &mut component.payload else { return };
        let live =
            |key: StableKey| Some(key) == spawning || self.storage.entities.contains_key(&key);
        value.remap(&mut |reference: &mut EntityRef| {
            let Some(key) = reference.get() else { return };
            let resolved = match (key, placeholders) {
                (StableKey::Placeholder(index), Some(placeholders)) => {
                    placeholders.get(index as usize).copied().flatten().filter(|&key| live(key))
                }
                (StableKey::Placeholder(_), None) => None,
                (key, Some(_)) => Some(self.canonical(key)).filter(|&key| live(key)),
                (key, None) => Some(self.canonical(key)),
            };
            if resolved.is_none() {
                tracing::debug!(target: "casein", ?key, "a reference to a missing entity is written empty");
            }
            *reference = EntityRef::from(resolved);
        });
    }

    /// A value of `info` to store directly, validated, with every bound `Net` key in its
    /// reference fields rewritten to the origin key. A placeholder is refused by name before
    /// validation: it is not a valid reference outside a command buffer.
    fn canonical_bytes(&self, info: &TypeInfo, mut bytes: Vec<u8>) -> Result<Vec<u8>> {
        if let Some(field) = placeholder_field(info, &bytes) {
            return Err(invalid(format!(
                "placeholder reference in `{field}` of `{}`: a placeholder names an entity only \
                 inside its command buffer",
                info.name()
            )));
        }
        info.validate(&bytes)?;
        for (_, key) in entity_refs(info, &bytes) {
            let origin = self.canonical(key);
            if origin != key {
                rewrite_refs(info, &mut bytes, key, origin);
            }
        }
        Ok(bytes)
    }

    /// A region resource's bytes, as [`canonical_bytes`](Self::canonical_bytes) gives them.
    fn canonical_resource(&self, info: &'static TypeInfo, bytes: &[u8]) -> Result<AlignedBytes> {
        Ok(AlignedBytes::from_bytes(&self.canonical_bytes(info, bytes.to_vec())?))
    }

    /// The description of the registered component `component`: `NotFound` if it is not
    /// registered, `InvalidData` if it is a resource.
    fn component_info(&self, component: ComponentKey) -> Result<&'static TypeInfo> {
        let info = self.registry.get(component).ok_or_else(|| {
            not_found(format!("component {:#018x} is not registered", component.0))
        })?;
        if info.role() != TypeRole::Component {
            return Err(invalid(format!("`{}` is not a component", info.name())));
        }
        Ok(info)
    }

    /// `data` as a component value for a direct write, through the checks
    /// [`insert_erased`](Self::insert_erased) states, in its order, before the entity's.
    fn erased_value(&self, data: &ComponentData) -> Result<ErasedComponent> {
        let info = self.component_info(data.component)?;
        if is_worlds_own(info.key()) {
            return Err(invalid(format!("`{}` is written by the world only", info.name())));
        }
        let mut bytes = data.bytes.clone();
        // The length is checked here, so a short value is `InvalidData` before validation.
        info.zero_padding(&mut bytes)?;
        let bytes = self.canonical_bytes(info, bytes)?;
        Ok(ErasedComponent { info, payload: Payload::Bytes(bytes) })
    }

    /// The direct spawn behind [`spawn`](Self::spawn) and [`spawn_erased`](Self::spawn_erased):
    /// the key rules, references resolved for a direct write, every check before the stamp,
    /// so a refused spawn changes nothing.
    fn spawn_direct(
        &mut self,
        key: StableKey,
        region: RegionId,
        mut components: Vec<ErasedComponent>,
    ) -> Result<()> {
        if matches!(key, StableKey::Placeholder(_)) {
            return Err(invalid("a placeholder names an entity only inside its command buffer"));
        }
        let key = self.canonical(key);
        for component in &mut components {
            self.resolve_references(component, None, Some(key));
        }
        let row = self.spawn_row(key, region, components)?;
        let stamp = self.bump();
        self.push_spawned(key, region, row, stamp);
        Ok(())
    }

    /// Rewrites every stored reference to `from` into `to`, the same entity under its origin
    /// key: a `Net` key written before the bind that makes it an alias, for instance an
    /// owner loaded from another region's save. The bytes then match every other machine's;
    /// no row is stamped, since nothing about the entity changed.
    pub(crate) fn rekey_references(&mut self, from: StableKey, to: StableKey) {
        // Region resources are not indexed: every one with a reference field is visited, so
        // its bytes do not depend on whether it was inserted before or after the bind.
        for state in self.regions.values_mut() {
            for resource in state.resources.values_mut() {
                if has_entity_ref(resource.info) {
                    let mut bytes = resource.bytes.as_bytes().to_vec();
                    rewrite_refs(resource.info, &mut bytes, from, to);
                    resource.bytes = AlignedBytes::from_bytes(&bytes);
                }
            }
        }
        let Some(entries) = self.references.by_target.remove(&from) else { return };
        let mut rows: Vec<(StableKey, ComponentKey)> =
            entries.iter().map(|&(source, component, _)| (source, component)).collect();
        rows.dedup();
        for (source, component) in rows {
            let (Some(loc), Some(info)) = (self.storage.loc(source), self.registry.get(component))
            else {
                continue;
            };
            let Some(bytes) = self.storage.component(loc, component) else { continue };
            let mut bytes = bytes.to_vec();
            rewrite_refs(info, &mut bytes, from, to);
            self.storage.overwrite(loc, component, &bytes);
        }
        self.references.by_target.entry(to).or_default().extend(entries);
    }

    pub(crate) fn index_references(
        &mut self,
        source: StableKey,
        info: &TypeInfo,
        bytes: &[u8],
        add: bool,
    ) {
        for (field, target) in entity_refs(info, bytes) {
            let entry = (source, info.key(), field);
            if add {
                self.references.by_target.entry(target).or_default().insert(entry);
            } else if let Some(set) = self.references.by_target.get_mut(&target) {
                set.remove(&entry);
                if set.is_empty() {
                    self.references.by_target.remove(&target);
                }
            }
        }
    }

    /// Adds `source` to, or removes it from, the claim index if the component is
    /// `ClaimedBy`; the bytes have been validated. Called beside [`index_references`]
    /// wherever a component enters or leaves a row.
    ///
    /// [`index_references`]: Self::index_references
    fn index_claim(&mut self, source: StableKey, info: &TypeInfo, bytes: &[u8], add: bool) {
        if info.key() != ClaimedBy::KEY {
            return;
        }
        let claim: ClaimedBy = bytemuck::checked::pod_read_unaligned(bytes);
        if add {
            self.claims.entry(claim.player).or_default().insert(source);
        } else if let Some(set) = self.claims.get_mut(&claim.player) {
            set.remove(&source);
            if set.is_empty() {
                self.claims.remove(&claim.player);
            }
        }
    }

    /// Spawns `key` in `region` with `components`, whose references are already resolved.
    /// Adds `InRegion`, and `PredictedOrigin` for a `Predicted` key that lacks it.
    pub(crate) fn spawn_entity(
        &mut self,
        key: StableKey,
        region: RegionId,
        components: Vec<ErasedComponent>,
        stamp: Stamp,
    ) -> Result<()> {
        let row = self.spawn_row(key, region, components)?;
        self.push_spawned(key, region, row, stamp);
        Ok(())
    }

    /// The row [`spawn_entity`](Self::spawn_entity) would push, every check made and
    /// nothing changed: the components with `InRegion`, and `PredictedOrigin` for a
    /// `Predicted` key that lacks it, ascending by key and validated.
    fn spawn_row(
        &self,
        key: StableKey,
        region: RegionId,
        mut components: Vec<ErasedComponent>,
    ) -> Result<Vec<(&'static TypeInfo, Vec<u8>)>> {
        if !self.regions.contains_key(&region) {
            return Err(not_found("the region is not loaded"));
        }
        match key {
            StableKey::Placeholder(_) => {
                return Err(invalid("a placeholder names an entity only inside its buffer"));
            }
            key if self.storage.entities.contains_key(&key) => {
                return Err(invalid(format!("{key:?} is already live")));
            }
            _ => {}
        }
        components.push(ErasedComponent::of(InRegion { region }));
        if let StableKey::Predicted(predicted) = key
            && !components.iter().any(|component| component.key() == PredictedOrigin::KEY)
        {
            components.push(ErasedComponent::of(PredictedOrigin {
                player: predicted.player,
                input_tick: predicted.input_tick,
                ordinal: predicted.ordinal,
            }));
        }
        components.sort_by_key(ErasedComponent::key);
        let mut row = Vec::with_capacity(components.len());
        for (index, component) in components.iter().enumerate() {
            if index > 0 && components[index - 1].key() == component.key() {
                return Err(invalid(format!("`{}` is in the bundle twice", component.info.name())));
            }
            let info = self.registry.get(component.key()).ok_or_else(|| {
                not_found(format!("`{}` is not registered", component.info.name()))
            })?;
            info.validate(component.bytes())?;
            row.push((info, component.bytes().to_vec()));
        }
        Ok(row)
    }

    /// Pushes a row [`spawn_row`](Self::spawn_row) checked, stamped at `stamp`, and indexes
    /// and logs it.
    fn push_spawned(
        &mut self,
        key: StableKey,
        region: RegionId,
        row: Vec<(&'static TypeInfo, Vec<u8>)>,
        stamp: Stamp,
    ) {
        let values: Vec<RowValue> = row
            .into_iter()
            .map(|(info, bytes)| RowValue { info, bytes, stamp, column_tick: stamp })
            .collect();
        let infos: Vec<&'static TypeInfo> = values.iter().map(|value| value.info).collect();
        let archetype = self.storage.archetype(&infos);
        self.storage.push(archetype, key, &values);
        for value in &values {
            self.index_references(key, value.info, &value.bytes, true);
            self.index_claim(key, value.info, &value.bytes, true);
        }
        self.log.push(self.tick, StructuralChange::Spawned { key, region });
        if let StableKey::Predicted(predicted) = key {
            self.despawned_predicted.remove(&predicted);
        }
    }

    /// Writes a whole component of the live entity `key`, adding it if absent.
    pub(crate) fn write_component(
        &mut self,
        key: StableKey,
        component: ErasedComponent,
        stamp: Stamp,
    ) -> Result<()> {
        let info = self.checked_info(&component)?;
        let loc = self.storage.loc(key).ok_or_else(|| not_found("no such entity"))?;
        if let Some(old) = self.storage.component(loc, info.key()) {
            let old = old.to_vec();
            self.index_references(key, info, &old, false);
            self.index_claim(key, info, &old, false);
            self.storage.write(loc, info.key(), component.bytes(), stamp);
        } else {
            let (_, mut values) = self.storage.take(key).expect("the entity is live");
            values.push(RowValue {
                info,
                bytes: component.bytes().to_vec(),
                stamp,
                column_tick: stamp,
            });
            values.sort_by_key(|value| value.info.key());
            let infos: Vec<&'static TypeInfo> = values.iter().map(|value| value.info).collect();
            let archetype = self.storage.archetype(&infos);
            self.storage.push(archetype, key, &values);
            self.log.push(self.tick, StructuralChange::Added { key, component: info.key() });
        }
        self.index_references(key, info, component.bytes(), true);
        self.index_claim(key, info, component.bytes(), true);
        Ok(())
    }

    /// The registry's description of `component`, whose bytes it validates: `NotFound` if it
    /// is not registered, `InvalidData` if the bytes are not a valid value.
    fn checked_info(&self, component: &ErasedComponent) -> Result<&'static TypeInfo> {
        let info = self
            .registry
            .get(component.key())
            .ok_or_else(|| not_found(format!("`{}` is not registered", component.info.name())))?;
        info.validate(component.bytes())?;
        Ok(info)
    }

    /// Removes a component of the live entity `key`, returning its bytes if it had it.
    pub(crate) fn remove_component(
        &mut self,
        key: StableKey,
        component: ComponentKey,
    ) -> Option<Vec<u8>> {
        let loc = self.storage.loc(key)?;
        self.storage.component(loc, component)?;
        let (_, mut values) = self.storage.take(key).expect("the entity is live");
        let index = values.iter().position(|value| value.info.key() == component)?;
        let removed = values.remove(index);
        let infos: Vec<&'static TypeInfo> = values.iter().map(|value| value.info).collect();
        let archetype = self.storage.archetype(&infos);
        self.storage.push(archetype, key, &values);
        self.index_references(key, removed.info, &removed.bytes, false);
        self.index_claim(key, removed.info, &removed.bytes, false);
        self.log.push(self.tick, StructuralChange::Removed { key, component });
        Some(removed.bytes)
    }

    /// Despawns the live entity `root` and everything it owns.
    pub(crate) fn despawn_tree(&mut self, root: StableKey) {
        for key in self.ownership_tree(root) {
            self.despawn_one(key);
        }
    }

    fn despawn_one(&mut self, key: StableKey) {
        let Some(loc) = self.storage.loc(key) else { return };
        let region = self.region_at(loc);
        let (_, values) = self.storage.take(key).expect("the entity is live");
        for value in &values {
            self.index_references(key, value.info, &value.bytes, false);
            self.index_claim(key, value.info, &value.bytes, false);
        }
        self.log.push(self.tick, StructuralChange::Despawned { key, region });
        if let StableKey::Predicted(predicted) = key {
            self.despawned_predicted.insert(predicted, self.tick);
        }
        // A bound `Net` key whose own entity is gone (one a restore revived) names the
        // predicted entity again: the references to it go to the origin key, as the bind
        // left them.
        if let StableKey::Net(net) = key
            && let Some(&predicted) = self.aliases.by_net.get(&net)
        {
            self.rekey_references(key, StableKey::Predicted(predicted));
        }
    }

    /// Moves the live entity `root` and everything it owns to the loaded `region`.
    pub(crate) fn move_tree(&mut self, root: StableKey, region: RegionId, stamp: Stamp) {
        for key in self.ownership_tree(root) {
            let loc = self.storage.loc(key).expect("the tree holds live entities");
            let from = self.region_at(loc);
            if from != region {
                let bytes = bytemuck::bytes_of(&InRegion { region }).to_vec();
                self.storage.write(loc, InRegion::KEY, &bytes, stamp);
                self.log.push(self.tick, StructuralChange::Moved { key, from, to: region });
            }
        }
    }
}

/// Iterates entities by origin key.
pub struct Entities<'w> {
    keys: std::vec::IntoIter<StableKey>,
    _world: PhantomData<&'w ()>,
}

impl Iterator for Entities<'_> {
    type Item = StableKey;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys.next()
    }
}

/// Iterates the entities one player has claimed, by origin key, from
/// [`World::claimed`].
pub struct Claimed<'w> {
    keys: Option<std::collections::btree_set::Iter<'w, StableKey>>,
}

impl Iterator for Claimed<'_> {
    type Item = StableKey;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys.as_mut()?.next().copied()
    }
}

/// Iterates loaded regions.
pub struct Regions<'w> {
    ids: std::vec::IntoIter<RegionId>,
    _world: PhantomData<&'w ()>,
}

impl Iterator for Regions<'_> {
    type Item = RegionId;

    fn next(&mut self) -> Option<Self::Item> {
        self.ids.next()
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
    referrers: std::vec::IntoIter<Referrer>,
    _world: PhantomData<&'w ()>,
}

impl Iterator for Referrers<'_> {
    type Item = Referrer;

    fn next(&mut self) -> Option<Self::Item> {
        self.referrers.next()
    }
}

/// Iterates resources as bytes, in ascending key order.
pub struct ErasedResources<'w> {
    resources: std::vec::IntoIter<(ComponentKey, &'w [u8])>,
}

impl<'w> Iterator for ErasedResources<'w> {
    type Item = (ComponentKey, &'w [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        self.resources.next()
    }
}
