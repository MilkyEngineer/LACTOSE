//! What a reconciliation reports (architecture sections 9.4 and 22): the corrections it
//! made, how far they reached, how far the prediction had diverged, whether the world must
//! resync, and the counters the milestone 1 spike records as its baseline. Values, never
//! log lines: `app` measures time around the call and divides by what is reported here.

use lactose_core::{StableKey, Tick};

/// What one [`Predictor::reconcile`](crate::Predictor::reconcile) did.
#[derive(Clone, Debug, Default)]
pub struct Reconciled {
    /// The newest server tick reconciled by this call, or `None` when no frame was popped
    /// since the last call. Every slot at or before it has been compared with the server's
    /// facts for it, matched or corrected, or skipped as from before the ring's restart.
    pub confirmed: Option<Tick>,
    /// How many frames were reconciled.
    pub frames: u32,
    /// Every correction made, oldest first: the rollback point, how deep it was, why, which
    /// entities it touched and by how much.
    pub corrections: Vec<Correction>,
    /// How many ticks were re-simulated: from the oldest correction's tick to the present,
    /// once. Zero without a correction. `app` divides the time it measured around the
    /// call by it for the rollback cost per tick. Bounded by the ring depth, which `app`
    /// sized from its budget; it does not count against the catch-up cap (D122).
    pub resimulated: u32,
    /// Why the prediction world must resync, if it must: `app` then calls
    /// [`Predictor::resync`](crate::Predictor::resync) and requests a full frame from the
    /// server. Nothing was compared or corrected for the frames the reason names.
    pub resync: Option<ResyncReason>,
}

/// One rollback: the world was restored to the slot of `tick`, the facts for `tick` were
/// written there, the physics was told, and every tick after it was re-simulated.
#[derive(Clone, Debug)]
pub struct Correction {
    /// The tick the world was restored to.
    pub tick: Tick,
    /// How many ticks lay between `tick` and the present when the correction was made:
    /// what was re-simulated for it, before any later correction in the same call.
    pub depth: u32,
    /// What made the tick wrong.
    pub cause: CorrectionCause,
    /// The entities whose rows, presence, region or claim were written at `tick`, in
    /// ascending key order. Presentation smooths their visible jump; a predicted
    /// interaction with a proxy that was corrected here was presented tentatively until
    /// now (architecture section 9.4).
    pub entities: Vec<StableKey>,
    /// How far the prediction at `tick` was from the server's state.
    pub divergence: Divergence,
}

/// Why a tick was corrected.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CorrectionCause {
    /// A frame's facts for the tick differed from the record: a field, a component, an
    /// entity, a region or a claim.
    Mismatch,
    /// The server substituted the previous input for the local player's sample at the
    /// tick after this one (architecture section 9.2); the ring's sample for that tick was
    /// replaced by the substitute and the tick re-simulated from here.
    Substituted,
    /// The first delivered sample after a substitution had its edges repaired
    /// (`PlayerInput::after_substitute`); the ring's sample for the tick after this one
    /// was replaced by the repaired one and the tick re-simulated from here.
    Repaired,
}

/// How far a prediction was from the server's state at one tick: counts, since a
/// magnitude across fields of different units means nothing, and this crate computes no
/// float.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Divergence {
    /// How many entities differed: in a field, a component's presence, their presence,
    /// their region or their claim.
    pub entities: u32,
    /// How many fields differed, over every compared component of every entity.
    pub fields: u32,
}

/// Why the prediction world cannot reconcile and must be rebuilt from replicated state
/// (architecture section 9.4: "corrections deeper than the ring trigger a rebuild").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ResyncReason {
    /// A frame for a tick before the ring's oldest slot arrived, and not from before the
    /// ring's restart: the server's confirmation lags further behind the present than the
    /// ring is deep.
    TooOld {
        /// The frame's tick.
        frame: Tick,
        /// The ring's oldest tick at the time.
        oldest: Tick,
    },
    /// A frame for a tick after the present waits in the inbox: the prediction world has
    /// fallen behind the server, which the client's clock sync should never let happen
    /// (architecture section 9.2: beyond the catch-up cap it snaps its clock). A waiting
    /// frame at or before the present is not this: no step ran to pop it.
    Behind {
        /// The oldest waiting frame's tick after the present.
        server: Tick,
        /// The present tick.
        present: Tick,
    },
}

/// The counters since the predictor was built: what the milestone 1 netcode spike
/// records as the baseline milestone 4's threshold is judged against (architecture
/// section 22). Monotonic; `app` differences two readings for a rate and divides the
/// times it measures outside the boundary by the tick counts here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PredictionStats {
    /// Ticks predicted: one per [`Predictor::step`](crate::Predictor::step).
    pub ticks_predicted: u64,
    /// Ticks re-simulated by corrections.
    pub ticks_resimulated: u64,
    /// Frames reconciled, skipped ones included.
    pub frames_reconciled: u64,
    /// Corrections made: the correction rate is this over `frames_reconciled`, or over
    /// the time `app` measured.
    pub corrections: u64,
    /// Fields that differed, summed over every correction.
    pub mismatched_fields: u64,
    /// Entities that differed, summed over every correction.
    pub mismatched_entities: u64,
    /// Substituted ticks reported by the server.
    pub substitutions: u64,
    /// Predicted spawns of the local player the server never echoed, despawned.
    pub unechoed_spawns: u64,
    /// Resyncs reported.
    pub resyncs: u64,
    /// Slots taken without their physics part, because the physics snapshot failed after
    /// the step ran; a rollback to one restores the world alone.
    pub slots_without_physics: u64,
    /// The deepest correction, in ticks re-simulated for it.
    pub deepest_correction: u32,
}
