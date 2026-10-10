//! What a client receives, as the tick applies it (architecture sections 6, 9.2, 9.3 and
//! 9.4): one [`StagedFrame`] per server tick, built on the network thread, moved by `app`
//! into each client world's [`StagedInbox`] between runs, and applied inside the tick by
//! the [`StagedState`](lactose_sim::StagedState) hook of `lactose-net` in the replicated
//! world and of `lactose-predict` in the prediction world. [`RemoteSlot`] is the one
//! component this crate adds to a client world: the entity's slot in the interpolation
//! buffer, which the staged spawn carries.
//!
//! Boundary code may use this module: plain records, keys and ticks; no float, no clock,
//! and no allocation when a frame is read.

use std::collections::{BTreeMap, BTreeSet, VecDeque, btree_map};
use std::sync::Arc;

use lactose_bundle::ComponentData;
use lactose_core::{ComponentKey, NetId, PredictionKey, StableKey, Tick};
use lactose_ecs::RegionId;
use lactose_reflect::{component, value};
use lactose_sim::TimeScale;

use crate::relevance::RegionChange;

/// Everything a client received for one server tick, in the form the sync point applies.
///
/// Every keyed field is a map, so a frame cannot hold two entries for one key and the
/// order packets arrived in decides nothing; the README states the order the hook applies
/// the fields in. Rows live in one byte arena, so reading a frame allocates nothing. A
/// frame is shared between the replicated world's hook, the prediction world's hook and
/// `app` through an `Arc`, each reading the fields that are its own.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StagedFrame {
    /// The server tick the frame is for: no row in it is from a later tick, and it is the
    /// tick of the replicated world's step that applies it.
    pub tick: Tick,
    /// Whether this frame is a full relevance-set snapshot, the one message late join and
    /// resync share (architecture section 9.3): then `regions` names every relevant region
    /// (`app` unloads every other loaded region in the same `TickInput`) and `spawns`
    /// names every relevant entity with every replicated component, and the hook despawns
    /// every entity the frame does not spawn before it applies the spawns.
    pub full: bool,
    /// The server's time scale from this tick on, if it changed or the frame is full:
    /// `app` copies it into `TickInput::time_scale` and into its pacer.
    pub time_scale: Option<TimeScale>,
    /// The processed-input acknowledgement for the receiving player: the newest input
    /// tick the server has applied. `lactose-predict` confirms its ring up to it and
    /// destroys a predicted spawn the server never echoed once this passes the spawn's
    /// input tick (architecture section 7).
    pub acked_input: Option<Tick>,
    /// The input ticks of the receiving player for which the server substituted the
    /// previous input (architecture section 9.2): the client is told, so that
    /// `lactose-predict` re-simulates those ticks with the substitute.
    pub substituted: BTreeSet<Tick>,
    /// Regions that became relevant, irrelevant or received a new asset table, by region:
    /// `app` turns these into `TickInput::{loaded, unloaded, assets}` under `Directed`
    /// control; the hook does not read them.
    pub regions: BTreeMap<RegionId, RegionChange>,
    /// Entities the client sees for the first time, by the server's `NetId`, each with
    /// every replicated component.
    pub spawns: BTreeMap<NetId, StagedSpawn>,
    /// Entities the server destroyed, by origin key: the tombstones of architecture
    /// section 9.3. A frame never names a key here and in `spawns` or among its rows.
    pub despawns: BTreeMap<StableKey, Tombstone>,
    rows: BTreeMap<(StableKey, ComponentKey), RowSlot>,
    bytes: Vec<u8>,
}

/// Where one row's bytes lie in the frame's arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RowSlot {
    tick: Tick,
    exact: bool,
    start: u32,
    len: u32,
}

impl StagedFrame {
    /// An empty frame for `tick`.
    #[must_use]
    pub fn new(tick: Tick) -> Self {
        Self {
            tick,
            full: false,
            time_scale: None,
            acked_input: None,
            substituted: BTreeSet::new(),
            regions: BTreeMap::new(),
            spawns: BTreeMap::new(),
            despawns: BTreeMap::new(),
            rows: BTreeMap::new(),
            bytes: Vec::new(),
        }
    }

    /// An empty full snapshot for `tick`: [`full`](Self::full) set.
    #[must_use]
    pub fn resync(tick: Tick) -> Self {
        Self { full: true, ..Self::new(tick) }
    }

    /// Stages the component `component` of `key` as received for server tick `tick`:
    /// `bytes` is the component's whole image as laid out in memory, exact (the server's
    /// full-precision bytes, for an entity the receiving player has claimed) or
    /// dequantised. A row newer than the one held for `(key, component)` replaces it, an
    /// older or equal one is ignored and `false` returned, so the newest received state
    /// wins whatever order packets arrived in.
    pub fn push_row(
        &mut self,
        key: StableKey,
        component: ComponentKey,
        tick: Tick,
        exact: bool,
        bytes: &[u8],
    ) -> bool {
        todo!()
    }

    /// The staged row of `component` for `key`, if the frame holds one.
    #[must_use]
    pub fn row(&self, key: StableKey, component: ComponentKey) -> Option<StagedRow<'_>> {
        let slot = self.rows.get(&(key, component))?;
        Some(StagedRow {
            key,
            component,
            tick: slot.tick,
            exact: slot.exact,
            bytes: self.slice(slot),
        })
    }

    /// Every staged row, in ascending (origin key, component key) order.
    #[must_use]
    pub fn rows(&self) -> StagedRows<'_> {
        StagedRows { entries: self.rows.iter(), bytes: &self.bytes }
    }

    /// How many rows the frame holds.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Whether the frame carries nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.full
            && self.time_scale.is_none()
            && self.acked_input.is_none()
            && self.substituted.is_empty()
            && self.regions.is_empty()
            && self.spawns.is_empty()
            && self.despawns.is_empty()
            && self.rows.is_empty()
    }

    fn slice(&self, slot: &RowSlot) -> &[u8] {
        &self.bytes[slot.start as usize..(slot.start + slot.len) as usize]
    }
}

/// One staged row: a whole component image for one entity, as received.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StagedRow<'a> {
    /// The entity's origin key.
    pub key: StableKey,
    /// The component.
    pub component: ComponentKey,
    /// The server tick the value is from.
    pub tick: Tick,
    /// Whether `bytes` are the server's full-precision bytes (an entity the receiving
    /// player has claimed, architecture section 9.4), or a dequantised image.
    pub exact: bool,
    /// The component's bytes, exactly as the component is laid out in memory.
    pub bytes: &'a [u8],
}

/// Iterates a frame's rows in ascending (origin key, component key) order.
pub struct StagedRows<'a> {
    entries: btree_map::Iter<'a, (StableKey, ComponentKey), RowSlot>,
    bytes: &'a [u8],
}

impl<'a> Iterator for StagedRows<'a> {
    type Item = StagedRow<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (&(key, component), slot) = self.entries.next()?;
        Some(StagedRow {
            key,
            component,
            tick: slot.tick,
            exact: slot.exact,
            bytes: &self.bytes[slot.start as usize..(slot.start + slot.len) as usize],
        })
    }
}

/// An entity the client sees for the first time, keyed by its `NetId` in
/// [`StagedFrame::spawns`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StagedSpawn {
    /// The entity's `Predicted` origin key, if its origin was a predicted spawn
    /// (architecture section 7); the hook spawns under [`key`](Self::key) and then binds
    /// the `NetId` to it with `World::bind`. For the receiving player's own prediction
    /// this is the bind `lactose-predict` waits for.
    pub origin: Option<PredictionKey>,
    /// The region the entity resides in, which the client has loaded or loads in the same
    /// frame.
    pub region: RegionId,
    /// The entity's slot in the client's interpolation buffer, allocated by the network
    /// thread, which the hook writes as [`RemoteSlot`]; `None` for an entity the
    /// receiving player has claimed, which the prediction world presents.
    pub slot: Option<InterpolationSlot>,
    /// Every replicated component of the entity, in ascending key order, exact or
    /// dequantised as the rows are.
    pub components: Vec<ComponentData>,
}

impl StagedSpawn {
    /// The key the hook spawns under: the origin key if there is one, else the `NetId`.
    #[must_use]
    pub fn key(&self, net: NetId) -> StableKey {
        self.origin.map_or(StableKey::Net(net), StableKey::Predicted)
    }
}

/// A tombstone (architecture section 9.3), keyed by the entity's origin key in
/// [`StagedFrame::despawns`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tombstone {
    /// The region the entity resided in.
    pub region: RegionId,
    /// The server tick at which it was destroyed.
    pub tick: Tick,
}

/// The frames waiting to be applied in one client world: a service resource `app` fills
/// between runs from the network thread's output, and the world's `StagedState` hook
/// drains inside the tick with [`pop_through`](Self::pop_through). No lock and no clock:
/// the two sides never run at once, since `app` owns the simulation between runs.
#[derive(Clone, Debug, Default)]
pub struct StagedInbox {
    frames: VecDeque<Arc<StagedFrame>>,
}

impl StagedInbox {
    /// An empty inbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `frame`. Frames are queued in ascending tick order: a frame whose tick is
    /// not after the newest held is refused and `false` returned.
    pub fn push(&mut self, frame: Arc<StagedFrame>) -> bool {
        todo!()
    }

    /// The oldest queued frame whose tick is not after `tick`, removed from the inbox; or
    /// `None` when the oldest frame is for a later tick or nothing is queued. The hook
    /// calls it until `None`, so a frame delayed by loss applies at the first sync point
    /// after it arrives.
    pub fn pop_through(&mut self, tick: Tick) -> Option<Arc<StagedFrame>> {
        todo!()
    }

    /// The tick of the newest queued frame.
    #[must_use]
    pub fn newest_tick(&self) -> Option<Tick> {
        self.frames.back().map(|frame| frame.tick)
    }

    /// How many frames are queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// An entity's slot in the client's interpolation buffer
/// ([`InterpolationBuffer`](crate::interpolation::InterpolationBuffer)): an index and the
/// generation the slot was allocated at, so a slot reused for another entity reads as
/// missing under the old value.
#[value]
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct InterpolationSlot {
    /// The slot index, below the buffer's capacity.
    #[field(id = 1)]
    pub index: u32,
    /// The allocation generation, never zero.
    #[field(id = 2)]
    pub generation: u32,
}

/// The interpolation slot of a remote entity in a client's replicated world, written by
/// the `StagedState` hook from [`StagedSpawn::slot`] when the entity is spawned, read by
/// extract so the presentation schedule knows which slot to sample for the entity's render
/// slot (architecture section 5.3), and removed with the row. Client-only, `derived` and
/// `extracted`; never on the server, never replicated, never written by game code.
#[component(key = "net::remote_slot", derived, extracted)]
#[derive(PartialEq, Eq, Debug)]
pub struct RemoteSlot {
    /// The slot.
    #[field(id = 1)]
    pub slot: InterpolationSlot,
}
