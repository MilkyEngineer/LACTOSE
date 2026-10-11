//! The rollback ring (architecture sections 9.4 and 10): one slot per predicted tick,
//! holding everything a return to that tick needs and everything a re-simulation of the
//! next tick needs, the newest slot being the present.
//!
//! The slot is a private type: what it holds is stated here and in the README, how it is
//! held is the implementation's, so that copy-on-write snapshots (COW, if the milestone 1
//! measurement calls for them) change no signature.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use lactose_core::{ComponentKey, PlayerId, StableKey, Tick};
use lactose_ecs::{RegionId, World, WorldSnapshot};
use lactose_net_api::StagedFrame;
use lactose_physics_api::PhysicsSnapshot;
use lactose_sim::TickInput;

/// The slots of the last [`depth`](Self::depth) predicted ticks, oldest first, the newest
/// being the present tick of the world. A slot older than the depth is dropped when a
/// newer one is taken, and the oldest slot's tick is the world's rollback horizon
/// (`World::set_rollback_horizon`): a frame for a tick before it cannot be reconciled and
/// is a resync, unless it is from before the ring's restart. After a rebuild (a full
/// frame) or a `resync` the ring starts again at one slot, [`rebuilt_at`](Self::rebuilt_at).
pub struct Ring {
    slots: VecDeque<Slot>,
    depth: u32,
    /// The tick the ring last restarted at, while that slot is still held: a frame for an
    /// earlier tick is from before the restart, and its facts and substitutions are
    /// skipped rather than reconciled.
    rebuilt_at: Option<Tick>,
}

impl Ring {
    /// How many slots the ring keeps at most.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// How many slots the ring holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the ring holds no slot, which is so only before the predictor's first slot
    /// or between `resync` and its fresh slot.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The oldest tick a rollback can return to: the horizon.
    #[must_use]
    pub fn oldest(&self) -> Option<Tick> {
        self.slots.front().map(|slot| slot.tick)
    }

    /// The newest slot's tick: the present, between runs.
    #[must_use]
    pub fn newest(&self) -> Option<Tick> {
        self.slots.back().map(|slot| slot.tick)
    }

    /// The tick the ring last restarted at (the step that applied a full frame, or a
    /// `resync`), while the ring still holds that slot; `None` once it has been dropped or
    /// if the ring never restarted.
    #[must_use]
    pub fn rebuilt_at(&self) -> Option<Tick> {
        self.rebuilt_at
    }

    /// What every slot weighs together: the sum of [`slot_bytes`](Self::slot_bytes) over
    /// the ring, which milestone 1 measures against the prediction world's budget and
    /// which decides whether copy-on-write snapshots are built.
    #[must_use]
    pub fn bytes(&self) -> SlotBytes {
        self.slots.iter().map(Slot::bytes).fold(SlotBytes::default(), |sum, slot| SlotBytes {
            world: sum.world + slot.world,
            physics: sum.physics + slot.physics,
            record: sum.record + slot.record,
        })
    }

    /// What the slot of `tick` weighs, or `None` if the ring holds no slot for it.
    #[must_use]
    pub fn slot_bytes(&self, tick: Tick) -> Option<SlotBytes> {
        self.slot(tick).map(Slot::bytes)
    }
}

/// What the predictor does to the ring. Slots are consecutive ticks, oldest first: a slot is
/// pushed only for the tick after the newest, and the ring restarts empty.
impl Ring {
    pub(crate) fn new(depth: u32) -> Self {
        Self { slots: VecDeque::new(), depth, rebuilt_at: None }
    }

    /// The index of `tick`'s slot.
    fn index(&self, tick: Tick) -> Option<usize> {
        let oldest = self.oldest()?;
        let index = usize::try_from(tick.0.wrapping_sub(oldest.0)).ok()?;
        (index < self.slots.len()).then_some(index)
    }

    pub(crate) fn slot(&self, tick: Tick) -> Option<&Slot> {
        self.slots.get(self.index(tick)?)
    }

    pub(crate) fn slot_mut(&mut self, tick: Tick) -> Option<&mut Slot> {
        let index = self.index(tick)?;
        self.slots.get_mut(index)
    }

    /// Appends `slot`, whose tick is the one after the newest (or any, to an empty ring),
    /// then drops the oldest beyond the depth.
    pub(crate) fn push(&mut self, slot: Slot) {
        debug_assert!(
            self.newest().is_none_or(|newest| newest.0.wrapping_add(1) == slot.tick.0),
            "slots are consecutive ticks"
        );
        self.slots.push_back(slot);
        self.trim();
    }

    /// Replaces the slot of `slot`'s tick, which the ring holds: a correction's re-take, a
    /// re-simulated tick.
    pub(crate) fn replace(&mut self, slot: Slot) {
        if let Some(held) = self.slot_mut(slot.tick) {
            *held = slot;
        }
    }

    /// Drops every slot after `tick`.
    pub(crate) fn truncate_after(&mut self, tick: Tick) {
        if let Some(index) = self.index(tick) {
            self.slots.truncate(index + 1);
        }
    }

    /// Drops every slot before `tick`.
    pub(crate) fn drop_before(&mut self, tick: Tick) {
        if let Some(index) = self.index(tick) {
            self.slots.drain(..index);
        }
        self.forget_rebuild();
    }

    /// Restarts the ring at the slot of `tick`, if it holds one.
    pub(crate) fn rebuild_at(&mut self, tick: Tick) {
        self.drop_before(tick);
        self.rebuilt_at = self.index(tick).map(|_| tick);
    }

    pub(crate) fn set_depth(&mut self, depth: u32) {
        self.depth = depth;
        self.trim();
    }

    /// Empties the ring.
    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.rebuilt_at = None;
    }

    fn trim(&mut self) {
        let depth = usize::try_from(self.depth).unwrap_or(usize::MAX);
        while self.slots.len() > depth {
            self.slots.pop_front();
        }
        self.forget_rebuild();
    }

    /// `rebuilt_at` names a slot the ring still holds, or nothing.
    fn forget_rebuild(&mut self) {
        if self.rebuilt_at.is_some_and(|tick| self.index(tick).is_none()) {
            self.rebuilt_at = None;
        }
    }
}

/// The size of a slot, or of the ring, by part.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SlotBytes {
    /// The world snapshot's bytes (`WorldSnapshot::size_in_bytes`).
    pub world: usize,
    /// The physics snapshot's bytes (`PhysicsSnapshot::size_in_bytes`): the backend's
    /// whole world, the part that decides how many statics a prediction world may hold;
    /// zero for a slot taken without one.
    pub physics: usize,
    /// The record's bytes: every replicated component of every claimed entity, as copied
    /// after the tick, plus the frames held for replay.
    pub record: usize,
}

/// One predicted tick, as a rollback returns to it and a re-simulation re-runs it.
pub(crate) struct Slot {
    /// The tick the world was at when the slot was taken.
    pub(crate) tick: Tick,
    /// The world after the tick: every column with its stamps, the entity table, the
    /// resources, the regions, the input-tick table, the relationship index.
    pub(crate) world: WorldSnapshot,
    /// The physics world after the tick and the sync's pending work, if the world has
    /// physics and the snapshot succeeded; `None` otherwise, and a restore to this slot
    /// restores the world alone. A slot taken again right after a restore from it (a
    /// correction's re-take) keeps `None` when it had none: the physics was not restored
    /// to this tick, and a snapshot then would hold a later tick's solver state.
    pub(crate) physics: Option<PhysicsSnapshot>,
    /// The frame that produced the tick, as `app` handed it to the step: the local
    /// player's sample (replaced when the server reports a substitution or the repair
    /// after one), the region bundles and tables, the time scale. Its `failures` are
    /// stripped when it is re-simulated, since the first application stamped them.
    pub(crate) input: TickInput,
    /// The staged frames the hook popped in the step for this tick, oldest first: the
    /// proxies' per-tick history (section 10), the binds, the spawns of proxies, the moves,
    /// removals and tombstones, replayed by the hook when the tick is re-simulated.
    pub(crate) frames: Vec<Arc<StagedFrame>>,
    /// Every entity the local player had claimed after the tick, with its region and the
    /// bytes of each of its replicated components (`World::get_erased` per entity and
    /// type), which a frame's exact rows for this tick are compared against.
    pub(crate) record: BTreeMap<StableKey, EntityRecord>,
    /// The regions loaded after the tick: an entity a fact names and the record lacks is
    /// re-spawned only into one of these.
    pub(crate) regions: Vec<RegionId>,
    /// The record's bytes and the frames' payload bytes, counted when the slot was taken.
    pub(crate) record_bytes: usize,
}

impl Slot {
    /// The slot of `world`'s current tick, between runs: the world snapshot, then the
    /// physics snapshot if `physics` (`None`, logged, if it fails), and the record of every
    /// entity `player` has claimed with each of the `recorded` components it has, read
    /// through `World::get_erased`.
    pub(crate) fn take(
        world: &World,
        player: PlayerId,
        recorded: &[ComponentKey],
        physics: bool,
        input: TickInput,
        frames: Vec<Arc<StagedFrame>>,
    ) -> Self {
        let snapshot = world.snapshot();
        let physics = physics.then(|| lactose_physics_api::snapshot(world)).and_then(|taken| {
            taken
                .inspect_err(|error| {
                    tracing::warn!(
                        target: "predict",
                        tick = world.tick().0,
                        %error,
                        "the physics snapshot failed: the slot restores the world alone"
                    );
                })
                .ok()
        });
        let mut record_bytes = frames.iter().map(|frame| frame_bytes(frame)).sum();
        let record: BTreeMap<StableKey, EntityRecord> = world
            .claimed(player)
            .filter_map(|key| {
                let region = world.region_of(key)?;
                let components: BTreeMap<ComponentKey, Vec<u8>> = recorded
                    .iter()
                    .filter_map(|&component| {
                        Some((component, world.get_erased(key, component)?.to_vec()))
                    })
                    .collect();
                Some((key, EntityRecord { region, components }))
            })
            .collect();
        record_bytes += record
            .values()
            .flat_map(|entry| entry.components.values())
            .map(Vec::len)
            .sum::<usize>();
        Self {
            tick: world.tick(),
            world: snapshot,
            physics,
            input,
            frames,
            record,
            regions: world.regions().collect(),
            record_bytes,
        }
    }

    fn bytes(&self) -> SlotBytes {
        SlotBytes {
            world: self.world.size_in_bytes(),
            physics: self.physics.as_ref().map_or(0, PhysicsSnapshot::size_in_bytes),
            record: self.record_bytes,
        }
    }
}

/// What the record holds of one claimed entity after a tick.
pub(crate) struct EntityRecord {
    pub(crate) region: RegionId,
    pub(crate) components: BTreeMap<ComponentKey, Vec<u8>>,
}

/// A frame's payload: its rows' bytes and its spawns' components' bytes.
fn frame_bytes(frame: &StagedFrame) -> usize {
    let rows: usize = frame.rows().map(|row| row.bytes.len()).sum();
    let spawns: usize = frame
        .spawns
        .values()
        .flat_map(|spawn| &spawn.components)
        .map(|data| data.bytes.len())
        .sum();
    rows + spawns
}
