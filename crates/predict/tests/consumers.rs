//! Compile-only consumer sketches against the facade: what the client's `app`, a game
//! crate, presentation and the milestone 1 spike write, type-checked and never run (every
//! deciding body of the crate is `todo!()`). They pin the surface the README promises: a
//! change that breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{Executor, Mut, Res, SystemDescriptor, World, WorldKind, system};
use lactose_input::{PlayerInput, PlayerInputApplier};
use lactose_math::Scalar;
use lactose_net_api::{StagedFrame, StagedInbox};
use lactose_physics_api::{Transform, Velocity};
use lactose_predict::{
    Correction, DEFAULT_DEPTH, LocalPlayer, PredictError, PredictionStats, Predictor, Reconciled,
    ResyncReason, SlotBytes, check_prediction_reads,
};
use lactose_reflect::{Registry, component};
use lactose_sim::{
    Registration, Simulation, SimulationBuilder, Stage, TickDelta, TickInput, TypeTable,
};
use lactose_task::TickPacer;

// ---- 1. A game crate's kernels, in both worlds, and what the build check says of them ----

/// Replicated: the server sends it exact for a claimed entity, so a prediction system may
/// read it.
#[component(key = "sketch::health", persistent, replicated)]
pub struct Health(#[field(id = 1)] pub u32);

/// Persistent and not replicated: never corrected, so a prediction system may not read it.
#[component(key = "sketch::diary", persistent)]
pub struct Diary(#[field(id = 1)] pub u32);

#[system(name = "sketch::fall", worlds = [server, prediction])]
fn fall(dt: Res<TickDelta>, vel: Mut<Velocity>, input: &PlayerInput) {
    vel.linear.y -= dt.0;
}

/// Names `Diary`: the check refuses it for the prediction world.
#[system(name = "sketch::reminisce", worlds = [server, prediction])]
fn reminisce(diary: &Diary, health: Mut<Health>) {
    health.0 += diary.0;
}

/// Names `Diary` too, but runs on the server only: not judged.
#[system(name = "sketch::write_diary", worlds = [server])]
fn write_diary(diary: Mut<Diary>) {
    diary.0 += 1;
}

pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Health>()?;
    types.register::<Diary>()?;
    lactose_input::register_types(types)?;
    lactose_physics_api::register_types(types)
}

pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&FALL).add(&REMINISCE).add(&WRITE_DIARY);
}

/// The check over a list of descriptors, as a test or the spike calls it; the one `app`
/// hands the prediction world's builder is the same function.
fn check(registry: &Registry) -> Result<()> {
    let systems: [&'static SystemDescriptor; 3] = [&FALL, &REMINISCE, &WRITE_DIARY];
    check_prediction_reads(registry, &systems)
}

// ---- 2. `app` on a client: build the prediction world beside the replicated one ----

fn build_prediction(player: PlayerId, start: Tick, entity: StableKey) -> Result<Predictor> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let simulation = SimulationBuilder::new(WorldKind::Prediction, types)
        .tick(start)
        .systems(lactose_input::register_systems)
        .systems(lactose_physics_api::register_systems)
        .systems(register_systems)
        .inputs(Box::new(PlayerInputApplier::new()))
        .build()?;
    Predictor::new(simulation, LocalPlayer { player, entity }, DEFAULT_DEPTH)
}

// ---- 3. `app` per frame: the timer root's loop around the predictor ----

/// What the client's `app` keeps beside the predictor.
struct Client {
    executor: Executor,
    predictor: Option<Predictor>,
    from_net: Receiver<Arc<StagedFrame>>,
    /// The re-simulation ticks the last `reconcile` reported, for the next `poll`.
    resimulated: u32,
    /// The lead the prediction world runs ahead of the server by, from clock sync.
    lead: u32,
}

/// One sample per new tick: the quantiser runs once per tick, outside the boundary.
fn local_sample(tick: Tick) -> PlayerInput {
    PlayerInput::IDLE
}

/// The fold of the due frames' time scale and region changes into this step's
/// `TickInput`: the same code the replicated world uses (`crates/net-api/README.md`).
fn fold_due(input: &mut TickInput, inbox: &StagedInbox, next: Tick) {
    for frame in inbox.due(next) {
        if let Some(scale) = frame.time_scale {
            input.time_scale = Some(scale);
        }
    }
}

/// The order of one frame: push, fold, poll with the re-simulation ticks of the previous
/// frame, step the new ticks (the first step's hook pops every due frame), reconcile what
/// the hook popped, run the passes, take the presentation events.
#[expect(
    clippy::disallowed_types,
    reason = "the timer root is `app`'s, outside the boundary; this sketch pins that it \
              compiles against `step`, `reconcile` and `passes`"
)]
async fn frame(client: &mut Client, pacer: &mut TickPacer) -> Result<()> {
    use std::time::Instant;
    let local = client.predictor.as_ref().expect("built").local();
    let mut predictor = client.predictor.take().expect("built");

    // Push first: a refused frame enters nothing.
    while let Ok(frame) = client.from_net.try_recv() {
        let inbox =
            predictor.simulation_mut().world_mut().service_mut::<StagedInbox>().expect("setup");
        let _ = inbox.push(frame);
    }

    let budget = pacer.poll(Instant::now(), client.resimulated);
    for i in 0..budget.ticks {
        let next = Tick(predictor.tick().0 + 1);
        let mut input = TickInput::default();
        if i == 0 {
            let inbox = predictor.simulation().world().service::<StagedInbox>().expect("setup");
            fold_due(&mut input, inbox, next);
        }
        input.inputs.insert(local.player, local_sample(next).to_player_sample(next));
        predictor = match predictor.step(&client.executor, input).await {
            Ok(predictor) => predictor,
            Err(PredictError { error, predictor: Some(kept) }) => {
                client.predictor = Some(kept);
                return Err(error);
            }
            Err(failure) => return Err(failure.into()),
        };
    }

    let started = Instant::now();
    let (predictor, reconciled) = predictor.reconcile(&client.executor).await?;
    let rollback_cost_per_tick = if reconciled.resimulated == 0 {
        None
    } else {
        Some(started.elapsed() / reconciled.resimulated)
    };
    client.resimulated = reconciled.resimulated;

    let mut predictor = predictor.passes(&client.executor).await?;
    let events = predictor.simulation_mut().world_mut().take_present_events();
    let _ = (events, rollback_cost_per_tick);
    presentation_hears(&reconciled);

    if let Some(reason) = reconciled.resync {
        let target = match reason {
            ResyncReason::TooOld { .. } => Tick(predictor.tick().0),
            ResyncReason::Behind { server, .. } => Tick(server.0 + client.lead),
        };
        pacer.resync(Instant::now());
        predictor.resync(target)?;
        // then: a `ResyncRequest` to the server, whose full frame rebuilds the world.
    }
    if budget.behind {
        pacer.resync(Instant::now());
    }
    client.predictor = Some(predictor);
    Ok(())
}

// ---- 4. Presentation (milestone 2): what it hears of a correction ----

fn presentation_hears(reconciled: &Reconciled) {
    for Correction { tick, depth, cause, entities, divergence } in &reconciled.corrections {
        let _smooth_these: &[StableKey] = entities;
        let _ = (tick, depth, cause, divergence.max_scalar_error);
    }
}

// ---- 5. The spike: the values it records, and the ring's weight ----

fn spike_record(predictor: &Predictor, seconds: f64) -> (f64, f64, Option<Scalar>, SlotBytes) {
    let PredictionStats { corrections, frames_reconciled, largest_scalar_error, .. } =
        predictor.stats();
    let per_minute = corrections as f64 / seconds * 60.0;
    let per_frame = corrections as f64 / frames_reconciled.max(1) as f64;
    (per_minute, per_frame, largest_scalar_error, predictor.ring().bytes())
}

/// Ring depth from the budget `app` measured: the frame re-simulation budget divided by
/// the per-tick cost, under `app`'s cap (architecture section 9.4).
fn size_ring(predictor: &mut Predictor, budget_us: u64, cost_per_tick_us: u64, cap: u32) {
    let depth = (budget_us / cost_per_tick_us.max(1)).min(u64::from(cap)) as u32;
    let _ = predictor.set_depth(depth.max(2));
}

// ---- 6. What the world holds that extract reads: the claimed set shadows the replicated
// world's copies (architecture section 5.3) ----

fn claimed_keys(world: &World, local: LocalPlayer) -> Vec<StableKey> {
    world.claimed(local.player).collect()
}

fn example_key() -> StableKey {
    StableKey::Net(NetId::from_bits(7).expect("non-zero"))
}

fn example_transform() -> Transform {
    Transform::IDENTITY
}

fn drive(simulation: Simulation) {
    let _ = simulation;
}
