//! The prediction world's staged-state hook (architecture sections 9.2 and 9.4;
//! `crates/sim/README.md`, "The input sync point"): what the prediction world does at the
//! input sync point with the frames the server sent, and what it records for the
//! reconciliation that runs between ticks.

use std::collections::BTreeSet;
use std::sync::Arc;

use lactose_core::{Result, Tick};
use lactose_ecs::World;
use lactose_net_api::StagedFrame;
use lactose_sim::StagedState;

use crate::predictor::LocalPlayer;

/// The `StagedState` hook of the prediction world, installed by
/// [`Predictor::new`](crate::Predictor::new) as the world's `dyn StagedState` service and
/// reinstalled by [`Predictor::resync`](crate::Predictor::resync);
/// `sim::apply_staged_state` calls it once per tick after the inputs have been applied.
///
/// It holds nothing: the player and entity it keeps the claim and the anchor on are the
/// record's, the one copy, which `set_local` replaces. In a predicted tick it pops every
/// frame due at the tick from the world's `StagedInbox` (`pop_through(tick)`); in a
/// re-simulated tick it applies the frames the ring's slot holds for the tick instead and
/// pops nothing. It writes the tick it ran at into the record, which is how a step knows
/// it ran. For each frame, oldest first, it applies what the README states under "The
/// hook": binds, the spawns of proxies as upserts, the proxies' rows as this tick's
/// position, moves, removals and tombstones; a full frame whole, as a rebuild. It writes
/// nothing of a claimed entity that a delta frame carries: those are facts for the
/// reconciliation, which it records with the tick it popped the frame at. Before and after
/// the frames it keeps `ClaimedBy` for the local player and `StreamingAnchor` on the
/// player's own entity while it is live, and the anchor on no other entity; and it
/// despawns a predicted spawn of the local player that the server never echoed once a
/// frame's acknowledgement is past its input tick. An entry that fails is logged and
/// skipped, as the replicated world's hook does; the tick runs.
pub struct PredictionHook {
    _private: (),
}

impl PredictionHook {
    /// The hook. Only the predictor makes one, since the record it fills is the
    /// predictor's.
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }
}

impl StagedState for PredictionHook {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        todo!()
    }
}

/// What the hook leaves for [`Predictor::reconcile`](crate::Predictor::reconcile) and what
/// the predictor leaves for the hook: a service resource of the prediction world under
/// this type, inserted by `Predictor::new` and replaced by `Predictor::resync`. Not state:
/// not snapshotted, not hashed.
pub(crate) struct PredictionRecord {
    /// The player and entity the hook keeps the claim and the anchor on: the only copy,
    /// replaced by `Predictor::set_local`.
    local: LocalPlayer,
    /// The tick the hook last ran at, or `None` before its first run: a step whose tick
    /// is not here resolves to an error, since the hook was replaced.
    ran_at: Option<Tick>,
    /// Every frame the hook popped since the last reconcile, in the order it popped them,
    /// each with the tick it was popped at.
    popped: Vec<Popped>,
    /// The frames the hook must apply in the next step instead of popping, during a
    /// re-simulation; `None` in a predicted tick.
    replay: Option<Vec<Arc<StagedFrame>>>,
    /// The ticks whose step despawned an unechoed predicted spawn, for the statistics.
    destroyed_at: BTreeSet<Tick>,
}

/// One frame as the hook popped it.
struct Popped {
    frame: Arc<StagedFrame>,
    /// The prediction tick the frame was popped at: at or after `frame.tick`, since the
    /// prediction world runs ahead of the server.
    applied_at: Tick,
}
