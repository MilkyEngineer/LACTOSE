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
#[derive(Clone, Debug)]
pub struct StagedFrame {
    /// The server tick the frame is for: no row in it is from a later tick. The hook
    /// applies it at the first sync point of its world whose tick is not before this
    /// ([`StagedInbox::pop_through`]), and `app` folds its time scale and region changes
    /// into the `TickInput` of that same step ([`StagedInbox::due`]).
    pub tick: Tick,
    /// Whether this frame is a full relevance-set snapshot, the one message late join and
    /// resync share (architecture section 9.3): then `regions` names every relevant region
    /// as `Relevant`, `spawns` names every relevant entity with every replicated component,
    /// and `time_scale` and `acked_input` are `Some`. `app` puts every loaded region in
    /// `TickInput::unloaded` of the same step, so each relevant region reloads from its
    /// message in that step (the sim applies unloads before loads); the hook applies the
    /// spawns as upserts and despawns nothing. From a resync request or the join until it
    /// stages the full frame the network thread stages nothing, and it clears its handle
    /// mirror and its interpolation slots when it stages that frame, not before.
    pub full: bool,
    /// The server's time scale from this tick on, if it changed or the frame is full:
    /// `app` copies it into `TickInput::time_scale` and into its pacer.
    pub time_scale: Option<TimeScale>,
    /// The processed-input acknowledgement for the receiving player: the newest input
    /// tick the server has applied. `lactose-predict` despawns a predicted spawn of the
    /// receiving player the server never echoed once this is after the spawn's input tick
    /// (architecture section 7); it confirms frames by their tick, not by this.
    pub acked_input: Option<Tick>,
    /// The input ticks of the receiving player for which the server substituted the
    /// previous input (architecture section 9.2): the client is told, so that
    /// `lactose-predict` re-simulates those ticks with the substitute where it differs
    /// from the sample it ran.
    pub substituted: BTreeSet<Tick>,
    /// Regions that became relevant, irrelevant or received a new asset table, by region:
    /// `app` turns these into `TickInput::{loaded, unloaded, assets}` under `Directed`
    /// control; the hook does not read them.
    pub regions: BTreeMap<RegionId, RegionChange>,
    /// Entities the client sees, by the server's `NetId`, each with every replicated
    /// component: seen for the first time, or again (a full frame, a change of claim, or a
    /// region becoming relevant whose rows the bundle already loaded). The hook upserts: a
    /// key already live has each component written whole and its [`RemoteSlot`] set or
    /// removed; a key that is not live is spawned and bound.
    pub spawns: BTreeMap<NetId, StagedSpawn>,
    /// Entities that moved between regions, by origin key, with the region they reside in
    /// now: from the gather's `Moved` entries (architecture section 6). The hook applies
    /// them after the spawns and before the rows, through `World::move_to_region`; owned
    /// entities follow their owner, so a later entry for one of them is a no-op. An entity
    /// moving into a region the client does not have leaves with a tombstone instead.
    pub moves: BTreeMap<StableKey, RegionId>,
    /// Replicated components the server removed from live entities, by (origin key,
    /// component key): from the gather's `Removed` entries. The hook applies them after
    /// the moves and before the rows, through `World::remove_erased`; a row for the same
    /// pair in the same frame then re-adds the component with the newer value. Never names a
    /// tombstoned key.
    pub removed: BTreeSet<(StableKey, ComponentKey)>,
    /// Entities that left the client's set, by origin key: the tombstones of architecture
    /// section 9.3, each saying whether the server destroyed the entity or it left the
    /// client's set alive (its region left it, or it moved into a region the client does
    /// not hold). A frame never names a key here and in `spawns`, in `removed` or among
    /// its rows.
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
            moves: BTreeMap::new(),
            removed: BTreeSet::new(),
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
    /// wins whatever order packets arrived in. A row from a tick after the frame's, or of
    /// another length than the row held, is refused the same way: a component's image has
    /// one size, which the receiver checks against the registry before it pushes the first
    /// row, so a held row is overwritten in place and the frame's arena never grows with
    /// the packets that arrive.
    #[must_use = "a refused row is not staged"]
    pub fn push_row(
        &mut self,
        key: StableKey,
        component: ComponentKey,
        tick: Tick,
        exact: bool,
        bytes: &[u8],
    ) -> bool {
        if after(tick, self.tick) {
            return false;
        }
        match self.rows.entry((key, component)) {
            btree_map::Entry::Occupied(mut entry) => {
                let slot = entry.get_mut();
                if !after(tick, slot.tick) || slot.len as usize != bytes.len() {
                    return false;
                }
                slot.tick = tick;
                slot.exact = exact;
                let start = slot.start as usize;
                self.bytes[start..start + bytes.len()].copy_from_slice(bytes);
            }
            btree_map::Entry::Vacant(entry) => {
                let (start, len) = append(&mut self.bytes, bytes);
                entry.insert(RowSlot { tick, exact, start, len });
            }
        }
        true
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
            && self.moves.is_empty()
            && self.removed.is_empty()
            && self.despawns.is_empty()
            && self.rows.is_empty()
    }

    fn slice(&self, slot: &RowSlot) -> &[u8] {
        &self.bytes[slot.start as usize..(slot.start + slot.len) as usize]
    }
}

/// Equal when every field and every row (key, component, tick, precision, bytes) is: where
/// a row's bytes lie in the arena is not part of the frame.
impl PartialEq for StagedFrame {
    fn eq(&self, other: &Self) -> bool {
        self.tick == other.tick
            && self.full == other.full
            && self.time_scale == other.time_scale
            && self.acked_input == other.acked_input
            && self.substituted == other.substituted
            && self.regions == other.regions
            && self.spawns == other.spawns
            && self.moves == other.moves
            && self.removed == other.removed
            && self.despawns == other.despawns
            && self.rows().eq(other.rows())
    }
}

impl Eq for StagedFrame {}

/// Appends `bytes` to an arena whose offsets are `u32`, returning where they lie.
pub(crate) fn append(arena: &mut Vec<u8>, bytes: &[u8]) -> (u32, u32) {
    let fits = |n: usize| u32::try_from(n).expect("a frame's byte arena is limited to 4 GiB");
    let (start, len) = (fits(arena.len()), fits(bytes.len()));
    fits(arena.len() + bytes.len());
    arena.extend_from_slice(bytes);
    (start, len)
}

/// Whether tick `tick` is after tick `than`, across the wrap, as the world orders ticks:
/// the two are taken to lie within 2^31 ticks of each other.
pub(crate) const fn after(tick: Tick, than: Tick) -> bool {
    tick.0 != than.0 && tick.0.wrapping_sub(than.0) < 1 << 31
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

/// An entity the client sees, keyed by its `NetId` in [`StagedFrame::spawns`]: a spawn
/// if the key is not live in the client's world, an upsert if it is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StagedSpawn {
    /// The entity's `Predicted` origin key, if its origin was a predicted spawn
    /// (architecture section 7); the hook spawns under [`key`](Self::key) and then binds
    /// the `NetId` to it with `World::bind`. For the receiving player's own prediction
    /// this is the bind `lactose-predict` waits for.
    pub origin: Option<PredictionKey>,
    /// The region the entity resides in, which the client has loaded or loads in the same
    /// step.
    pub region: RegionId,
    /// The entity's slot in the client's interpolation buffer, allocated by the network
    /// thread, which the hook writes as [`RemoteSlot`]; `None` for an entity the
    /// receiving player has claimed, which the prediction world presents. A change of
    /// claim is sent as a spawn of the live entity with the slot set or `None`, and the
    /// hook sets or removes `RemoteSlot` accordingly; `lactose-predict` reads the same
    /// spawn to start predicting the entity (`None`, claimed) or stop (`Some`, released).
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
/// [`StagedFrame::despawns`]: the entity left the client's set at `tick`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tombstone {
    /// The region the entity resided in on the server at `tick`: the one it was destroyed
    /// in, or the one it is alive in, which the client does not hold or is losing.
    pub region: RegionId,
    /// The server tick at which it left the client's set.
    pub tick: Tick,
    /// Why it left, which decides whether the hook may skip it.
    pub cause: TombstoneCause,
}

/// Why an entity left the client's set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TombstoneCause {
    /// The server destroyed the entity.
    Destroyed,
    /// The entity is alive on the server in [`Tombstone::region`]: that region left the
    /// client's set at the tombstone's tick, or the entity moved there, out of the client's
    /// set or while a tombstone of it was owed. The hook skips such a tombstone when the
    /// client's entity resides in that region, which settles it: the region's unload is
    /// still to come, or the server names the region and the entity enters again. A later
    /// move or destruction of the entity replaces the tombstone the server owes, so that it
    /// names where the server has the entity, and the client stages it under the same key
    /// even after the hook kept the entity.
    Left,
}

/// The frames waiting to be applied in one client world: a service resource `app` fills
/// between runs from the network thread's output, and the world's `StagedState` hook
/// drains inside the tick with [`pop_through`](Self::pop_through). No lock and no clock:
/// the two sides never run at once, since `app` owns the simulation between runs.
///
/// Which tick is which: the step for `tick` advances the world to `tick` and then runs
/// the hook with it; before that step, `Simulation::tick` is `tick - 1`. So `app`, between
/// runs, folds [`due`](Self::due) of the tick the next step will run, `Tick
/// (simulation.tick().0 + 1)`, never of `simulation.tick()`, which leaves out every frame
/// that arrived in time.
#[derive(Clone, Debug, Default)]
pub struct StagedInbox {
    frames: VecDeque<Arc<StagedFrame>>,
    /// The newest tick ever accepted, queued or already popped.
    accepted: Option<Tick>,
}

impl StagedInbox {
    /// An empty inbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `frame`. Frames are accepted in ascending tick order: a frame whose tick is
    /// not after the newest tick the inbox ever accepted, queued or popped, is refused and
    /// `false` returned, so no row can go back in time across frames. `app` pushes first
    /// and folds the frame's time scale and region changes into the `TickInput` only on
    /// acceptance ([`due`](Self::due)). Under the receiver rules of `lactose-net-host`
    /// (a packet is staged only if its tick is after the newest frame staged, and nothing
    /// is staged between a resync request or the join and its full frame) every frame the
    /// network thread hands over is accepted; a refusal is a defect in the receiver.
    #[must_use = "a refused frame's region changes and time scale must not enter the TickInput"]
    pub fn push(&mut self, frame: Arc<StagedFrame>) -> bool {
        if self.accepted.is_some_and(|accepted| !after(frame.tick, accepted)) {
            return false;
        }
        self.accepted = Some(frame.tick);
        self.frames.push_back(frame);
        true
    }

    /// The oldest queued frame whose tick is not after `tick`, removed from the inbox; or
    /// `None` when the oldest frame is for a later tick or nothing is queued. The hook
    /// calls it until `None` at the sync point of the step for `tick`, with the `tick` the
    /// hook received (the one the step advanced the world to), so a frame delayed by loss
    /// applies at the first sync point after it arrives.
    pub fn pop_through(&mut self, tick: Tick) -> Option<Arc<StagedFrame>> {
        // Queued ticks ascend, so the oldest frame decides.
        if after(self.frames.front()?.tick, tick) {
            return None;
        }
        self.frames.pop_front()
    }

    /// The queued frames whose tick is not after `tick`, oldest first, without removing
    /// them: exactly the frames [`pop_through`](Self::pop_through) will hand the hook in
    /// the step for `tick`, so `app` folds their time scale and region changes into that
    /// step's `TickInput` before the step. The step for `tick` advances the world to
    /// `tick`; before it, `Simulation::tick` is `tick - 1`, so the call between runs is
    /// `due(Tick(simulation.tick().0 + 1))`.
    #[must_use]
    pub fn due(&self, tick: Tick) -> DueFrames<'_> {
        DueFrames { frames: self.frames.iter(), through: tick }
    }

    /// The tick of the oldest queued frame that is after `tick`: the first frame that will
    /// not apply in the step for `tick`, or `None` when every queued frame is due by then.
    /// `lactose-predict` asks it with the present tick to learn the server is ahead.
    #[must_use]
    pub fn oldest_after(&self, tick: Tick) -> Option<Tick> {
        // Queued ticks ascend, so the frames due at `tick` are a prefix.
        self.frames.iter().map(|frame| frame.tick).find(|&queued| after(queued, tick))
    }

    /// Empties the inbox and forgets the newest tick it accepted, so the next frame pushed
    /// is accepted whatever its tick. A re-join that moves the world's tick with
    /// `World::set_tick` calls it beside that call: the new session's ticks may lie before
    /// the old session's, which [`push`](Self::push) would refuse.
    pub fn reset(&mut self) {
        self.frames.clear();
        self.accepted = None;
    }

    /// The newest tick ever accepted, queued or already popped.
    #[must_use]
    pub fn accepted_tick(&self) -> Option<Tick> {
        self.accepted
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

/// Iterates the queued frames due at a tick, oldest first.
pub struct DueFrames<'a> {
    frames: std::collections::vec_deque::Iter<'a, Arc<StagedFrame>>,
    through: Tick,
}

impl<'a> Iterator for DueFrames<'a> {
    type Item = &'a Arc<StagedFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        // Queued ticks ascend: the first frame after `through` ends the iteration, and a
        // peek through a copy of the iterator leaves it to be seen again.
        let frame = self.frames.clone().next()?;
        if after(frame.tick, self.through) {
            return None;
        }
        self.frames.next()
    }
}

/// An entity's slot in the client's interpolation buffer (`lactose-net-host`'s
/// `InterpolationBuffer`): an index and the generation the slot was allocated at, so a
/// slot reused for another entity reads as missing under the old value.
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
/// the `StagedState` hook from [`StagedSpawn::slot`] when the entity is spawned or
/// upserted, removed by the hook when a spawn carries `None` (the receiving player
/// claimed the entity), read by extract so the presentation schedule knows which slot to
/// sample for the entity's render slot (architecture section 5.3), and gone with the row.
/// Client-only, `derived` and `extracted`; never on the server, never replicated, never
/// written by game code.
#[component(key = "net::remote_slot", derived, extracted)]
#[derive(PartialEq, Eq, Debug)]
pub struct RemoteSlot {
    /// The slot.
    #[field(id = 1)]
    pub slot: InterpolationSlot,
}
