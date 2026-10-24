//! The client's prediction world (architecture sections 4, 9.4 and 10): the ring of
//! per-tick snapshots it rolls back through, the hook that applies what the server staged
//! at the input sync point, and the reconciliation that compares the server's exact rows
//! with what was predicted and re-simulates from the first difference.
//!
//! Foundation tier, inside the determinism boundary (architecture section 8.1): every key
//! is a `StableKey`, nothing here reads a clock, and every float goes through `lactose_math`
//! (the tolerance of D124). The `README.md`
//! is the contract: it states what the signatures cannot, and what a caller may rely on.
//!
//! # Where to start
//!
//! - [`Predictor`] owns the prediction [`Simulation`](lactose_sim::Simulation) and runs it
//!   by value as `sim` does: [`Predictor::step`] runs one tick ahead with the local
//!   player's sample and takes a ring slot; [`Predictor::reconcile`] compares every frame
//!   the hook popped since the last call and rolls back and re-simulates from the first
//!   difference; [`Predictor::passes`] runs the frame's passes for presentation.
//! - [`PredictionHook`] is the `StagedState` hook [`Predictor::new`] installs: it applies
//!   the staged frames inside the tick, moves the proxies along their history and records
//!   the claimed entities' exact rows for reconciliation.
//! - [`Ring`] is the rollback ring: what one slot holds, how deep it is, what it weighs.
//! - [`Reconciled`], [`Correction`], [`Divergence`] and [`PredictionStats`] (with
//!   [`Absorbed`]) are the values the milestone 1 spike records; nothing here is a log line.
//! - [`PredictionTolerance`] is how far a dynamic body's `Transform` and `Velocity` may
//!   differ from the server's and still agree; `app` sets it with
//!   [`Predictor::set_tolerance`].
//! - [`check_prediction_reads`] is the build check of D121: a prediction-world system may
//!   name only components the server corrects, and an exclusive one must be one of
//!   [`EXCLUSIVE_SYSTEMS`].
//! - [`StagedRegion`] is the one component of this crate, registered through
//!   [`register_types`]: where each proxy resided after the frames last placed it, which
//!   the hook's tombstone rule reads beside where the proxy resides now.
//!
//! ```no_run
//! use lactose_core::{NetId, PlayerId, StableKey, Tick};
//! use lactose_ecs::{Executor, WorldKind};
//! use lactose_predict::{DEFAULT_DEPTH, LocalPlayer, PredictError, Predictor};
//! use lactose_sim::{PlayerSample, SimulationBuilder, TickInput, TypeTable};
//!
//! # fn local_sample(tick: Tick) -> PlayerSample { unimplemented!() }
//! # fn main() -> lactose_core::Result<()> {
//! // What the client's `app` does once, after `JoinAccepted` names the player, the tick
//! // and the player's entity: the world built at the join tick plus the lead it runs
//! // ahead by, with every crate's registration, `lactose_net_api::setup` for the inbox
//! // among them.
//! let executor = Executor::builder().build()?;
//! let (join_tick, lead) = (Tick(100), 6);
//! let mut types = TypeTable::new();
//! lactose_input::register_types(&mut types)?;
//! lactose_net_api::register_types(&mut types)?;
//! lactose_predict::register_types(&mut types)?;
//! // ... every other crate's `register_types`, `setup` and `register_systems` ...
//! let simulation = SimulationBuilder::new(WorldKind::Prediction, types)
//!     .tick(Tick(join_tick.0 + lead))
//!     .setup(lactose_net_api::setup)
//!     .systems(lactose_input::register_systems)
//!     .systems(lactose_net_api::register_systems)
//!     .check(lactose_predict::check_prediction_reads)
//!     .inputs(Box::new(lactose_input::PlayerInputApplier::new()))
//!     .build()?;
//! let entity = StableKey::Net(NetId::from_bits(7).expect("non-zero"));
//! let local = LocalPlayer { player: PlayerId(1), entity };
//! let predictor = Predictor::new(simulation, local, DEFAULT_DEPTH)?;
//!
//! // One frame of the timer root with one new tick due: the frames `app` pushed into the
//! // inbox are popped by this step's hook; the step runs ahead with the local sample;
//! // `reconcile` compares what the hook popped and rolls back if it must; the passes
//! // extract for presentation. Every run hands the predictor back on a failure its world
//! // survived, so `app` matches rather than `?`. `app` measures `reconcile`'s time
//! // outside the boundary.
//! let next = Tick(predictor.tick().0 + 1);
//! let mut input = TickInput::default();
//! input.inputs.insert(local.player, local_sample(next));
//! let predictor = match executor.block_on(predictor.step(&executor, input)) {
//!     Ok(predictor) => predictor,
//!     Err(PredictError { error, predictor: Some(kept) }) => {
//!         // keep `kept`, resync it, report `error`
//!         # let _ = error;
//!         kept
//!     }
//!     Err(torn) => return Err(torn.into()),
//! };
//! let (predictor, reconciled) = executor.block_on(predictor.reconcile(&executor))?;
//! let resimulated = reconciled.resimulated; // the rollback cost's divisor
//! let mut predictor = executor.block_on(predictor.passes(&executor))?;
//! let events = predictor.simulation_mut().world_mut().take_present_events();
//! # let _ = (resimulated, events);
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

mod check;
mod compare;
mod fact;
mod hook;
mod predictor;
mod reconcile;
mod ring;
mod staged_region;

pub use check::{EXCLUSIVE_SYSTEMS, check_prediction_reads, prediction_may_name};
pub use compare::PredictionTolerance;
pub use hook::PredictionHook;
pub use predictor::{
    DEFAULT_DEPTH, LocalPlayer, PassesRun, PredictError, Predictor, ReconcileRun, StepRun,
};
pub use reconcile::{
    Absorbed, Correction, CorrectionCause, Divergence, PredictionStats, Reconciled, ResyncReason,
};
pub use ring::{Ring, SlotBytes};
pub use staged_region::{StagedRegion, register_types};

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the predictor is owned by the timer root, a `'static` task,
/// and its runs are driven there; the hook is what `sim` takes as its staged-state service;
/// the ring travels with the predictor; the outcomes and the statistics are plain values
/// a frame hands across threads; the default depth is a ring, not a slot; and the region
/// record is a component the world stores and snapshots, `derived` so it is never hashed
/// or saved and so a prediction system may name it (D121's check).
const _: () = {
    use lactose_reflect::{Component, Persistence, Reflect};
    use lactose_sim::StagedState;

    const fn send<T: Send>() {}
    const fn send_sync<T: Send + Sync>() {}
    const fn staged_state<T: StagedState>() {}
    const fn component<T: Component>() {}

    component::<StagedRegion>();
    send_sync::<StagedRegion>();
    assert!(
        matches!(StagedRegion::SCHEMA.info().persistence(), Persistence::Derived),
        "the region record is derived: restored by a rollback, never hashed or saved"
    );

    send::<Predictor>();
    send::<StepRun<'_>>();
    send::<ReconcileRun<'_>>();
    send::<PassesRun<'_>>();
    send::<PredictError>();
    send::<Ring>();
    send_sync::<PredictionHook>();
    send_sync::<LocalPlayer>();
    send_sync::<Reconciled>();
    send_sync::<Correction>();
    send_sync::<Divergence>();
    send_sync::<PredictionStats>();
    send_sync::<Absorbed>();
    send_sync::<PredictionTolerance>();
    send_sync::<ResyncReason>();
    send_sync::<SlotBytes>();
    staged_state::<PredictionHook>();

    assert!(DEFAULT_DEPTH >= 2, "a ring holds the present and at least one tick to return to");
};
