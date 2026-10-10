//! The rollback ring (architecture sections 9.4 and 10): one slot per predicted tick,
//! holding everything a return to that tick needs and everything a re-simulation of the
//! next tick needs, the newest slot being the present.
//!
//! The slot is a private type: what it holds is stated here and in the README, how it is
//! held is the implementation's, so that copy-on-write snapshots (COW, if the milestone 1
//! measurement calls for them) change no signature.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use lactose_core::{ComponentKey, StableKey, Tick};
use lactose_ecs::{RegionId, WorldSnapshot};
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
        todo!()
    }

    /// What the slot of `tick` weighs, or `None` if the ring holds no slot for it.
    #[must_use]
    pub fn slot_bytes(&self, tick: Tick) -> Option<SlotBytes> {
        todo!()
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
struct Slot {
    /// The tick the world was at when the slot was taken.
    tick: Tick,
    /// The world after the tick: every column with its stamps, the entity table, the
    /// resources, the regions, the input-tick table, the relationship index.
    world: WorldSnapshot,
    /// The physics world after the tick and the sync's pending work, if the world has
    /// physics and the snapshot succeeded; `None` otherwise, and a restore to this slot
    /// restores the world alone.
    physics: Option<PhysicsSnapshot>,
    /// The frame that produced the tick, as `app` handed it to the step: the local
    /// player's sample (replaced when the server reports a substitution or the repair
    /// after one), the region bundles and tables, the time scale. Its `failures` are
    /// stripped when it is re-simulated, since the first application stamped them.
    input: TickInput,
    /// The staged frames the hook popped in the step for this tick, oldest first: the
    /// proxies' per-tick history (section 10), the binds, the spawns of proxies, the moves,
    /// removals and tombstones, replayed by the hook when the tick is re-simulated.
    frames: Vec<Arc<StagedFrame>>,
    /// Every entity the local player had claimed after the tick, with its region and the
    /// bytes of each of its replicated components (`World::get_erased` per entity and
    /// type), which a frame's exact rows for this tick are compared against.
    record: BTreeMap<StableKey, EntityRecord>,
}

/// What the record holds of one claimed entity after a tick.
struct EntityRecord {
    region: RegionId,
    components: BTreeMap<ComponentKey, Vec<u8>>,
}
