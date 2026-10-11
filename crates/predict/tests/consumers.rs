//! Compile-only consumer sketches against the facade: what the client's `app`, a game
//! crate, presentation and the milestone 1 spike write, type-checked and never run (every
//! deciding body of the crate is `todo!()`). They pin the surface the README promises: a
//! change that breaks one is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use lactose_core::{NetId, PlayerId, Result, StableKey, SystemKey, Tick};
use lactose_ecs::{Executor, Mut, Res, SystemDescriptor, World, WorldKind, system};
use lactose_input::{PlayerInput, PlayerInputApplier};
use lactose_net_api::{StagedFrame, StagedInbox};
use lactose_physics_api::{PhysicsBackend, PhysicsConfig, Transform, Velocity};
use lactose_predict::{
    Correction, DEFAULT_DEPTH, EXCLUSIVE_SYSTEMS, LocalPlayer, PredictError, PredictionStats,
    Predictor, Reconciled, ResyncReason, SlotBytes, check_prediction_reads,
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

/// Exclusive and in no engine set: the check refuses it for the prediction world, since
/// `&mut World` shows no table.
#[system(name = "sketch::sweep", worlds = [server, prediction], exclusive)]
fn sweep(world: &mut World) {}

pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Health>()?;
    types.register::<Diary>()?;
    lactose_input::register_types(types)?;
    lactose_net_api::register_types(types)?;
    lactose_physics_api::register_types(types)
}

pub fn register_systems(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&FALL).add(&REMINISCE).add(&WRITE_DIARY);
    registration.stage(Stage::Post).add(&SWEEP);
}

/// The check over a list of descriptors, as a test or the spike calls it; the one `app`
/// hands the prediction world's builder is the same function.
fn check(registry: &Registry) -> Result<()> {
    let systems: [&'static SystemDescriptor; 4] = [&FALL, &REMINISCE, &WRITE_DIARY, &SWEEP];
    let _physics_sync_is_allowed =
        EXCLUSIVE_SYSTEMS.contains(&SystemKey::from_name("physics_api::sync"));
    check_prediction_reads(registry, &systems)
}

// ---- 2. `app` on a client: build the prediction world beside the replicated one ----

/// The backend crate's constructor, with the pool `app` built outside the boundary.
fn backend_with_pool() -> Box<dyn PhysicsBackend> {
    unimplemented!()
}

/// Built at the join tick plus the clock sync's lead, with every crate's registration:
/// `lactose_net_api::setup` for the inbox the hook pops, `lactose_physics_api::setup` for
/// the physics the ring snapshots.
fn build_prediction(
    player: PlayerId,
    join_tick: Tick,
    lead: u32,
    entity: StableKey,
) -> Result<Predictor> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    let backend = backend_with_pool();
    let config = PhysicsConfig { reserved_workers: 2, ..PhysicsConfig::default() };
    let simulation = SimulationBuilder::new(WorldKind::Prediction, types)
        .tick(Tick(join_tick.0 + lead))
        .setup(lactose_net_api::setup)
        .setup(move |world| lactose_physics_api::setup(world, backend, config))
        .systems(lactose_input::register_systems)
        .systems(lactose_net_api::register_systems)
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
    /// The lead the prediction world runs ahead of the server by, from clock sync.
    lead: u32,
    /// What the last `reconcile` reported, for the spike's cost per tick.
    resimulated: u32,
}

/// What a frame leaves `app` to do next.
enum Next {
    /// Carry on.
    Running,
    /// The predictor was handed back: resync it to `target`, request a full frame.
    Resync(Tick),
    /// The world is torn.
    Torn(lactose_core::Error),
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

/// The order of one frame: push, fold, poll (the cap is for catch-up alone, D122), step
/// the new ticks (the first step's hook pops every due frame), reconcile what the hook
/// popped, run the passes, take the presentation events. Every run that hands the
/// predictor back is matched, never `?`ed, so the world is kept.
#[expect(
    clippy::disallowed_types,
    reason = "the timer root is `app`'s, outside the boundary; this sketch pins that it \
              compiles against `step`, `reconcile` and `passes`"
)]
async fn frame(client: &mut Client, pacer: &mut TickPacer) -> Next {
    use std::time::Instant;
    let local = client.predictor.as_ref().expect("built").local();
    let mut predictor = client.predictor.take().expect("built");

    // Push first: a refused frame enters nothing.
    while let Ok(frame) = client.from_net.try_recv() {
        let inbox =
            predictor.simulation_mut().world_mut().service_mut::<StagedInbox>().expect("setup");
        let _ = inbox.push(frame);
    }

    let budget = pacer.poll(Instant::now());
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
                let target = Tick(kept.tick().0);
                client.predictor = Some(kept);
                return Next::Resync(target);
            }
            Err(PredictError { error, predictor: None }) => return Next::Torn(error),
        };
    }

    let started = Instant::now();
    let (predictor, reconciled) = match predictor.reconcile(&client.executor).await {
        Ok(done) => done,
        Err(PredictError { error, predictor: Some(kept) }) => {
            let target = Tick(kept.tick().0);
            client.predictor = Some(kept);
            return Next::Resync(target);
        }
        Err(PredictError { error, predictor: None }) => return Next::Torn(error),
    };
    let rollback_cost_per_tick = if reconciled.resimulated == 0 {
        None
    } else {
        Some(started.elapsed() / reconciled.resimulated)
    };
    client.resimulated = reconciled.resimulated;

    let mut predictor = match predictor.passes(&client.executor).await {
        Ok(predictor) => predictor,
        Err(PredictError { error, predictor: Some(kept) }) => {
            client.predictor = Some(kept);
            return Next::Running;
        }
        Err(PredictError { error, predictor: None }) => return Next::Torn(error),
    };
    let events = predictor.simulation_mut().world_mut().take_present_events();
    let failures = predictor.simulation_mut().lifecycle_mut().take_asset_failures();
    let _ = (events, failures, rollback_cost_per_tick);
    presentation_hears(&reconciled);

    let next = match reconciled.resync {
        Some(ResyncReason::TooOld { .. }) => Next::Resync(Tick(predictor.tick().0)),
        Some(ResyncReason::Behind { server, .. }) => Next::Resync(Tick(server.0 + client.lead)),
        None => Next::Running,
    };
    if budget.behind {
        // Catch-up alone owed more than the cap: the clock, never the world.
        pacer.resync(Instant::now());
    }
    client.predictor = Some(predictor);
    next
}

/// What `app` does with `Next::Resync`: the world to `target`, the pacer's clock, then a
/// `ResyncRequest` to the server, whose full frame rebuilds the world.
#[expect(
    clippy::disallowed_types,
    reason = "the timer root is `app`'s, outside the boundary; this sketch pins that it \
              compiles against `resync`"
)]
fn resync(client: &mut Client, pacer: &mut TickPacer, target: Tick) -> Result<()> {
    use std::time::Instant;
    pacer.resync(Instant::now());
    client.predictor.as_mut().expect("kept").resync(target)
}

/// The server named another entity for the player (a re-join after a load): between
/// runs, before the full frame that follows.
fn player_entity_changed(client: &mut Client, entity: StableKey) -> Result<()> {
    let predictor = client.predictor.as_mut().expect("kept");
    let player = predictor.local().player;
    predictor.set_local(LocalPlayer { player, entity })
}

// ---- 4. Presentation (milestone 2): what it hears of a correction ----

fn presentation_hears(reconciled: &Reconciled) {
    for Correction { tick, depth, cause, entities, divergence } in &reconciled.corrections {
        let _smooth_these: &[StableKey] = entities;
        let _ = (tick, depth, cause, divergence.entities, divergence.fields);
    }
}

// ---- 5. The spike: the values it records, and the ring's weight ----

fn spike_record(predictor: &Predictor, seconds: f64) -> (f64, f64, SlotBytes) {
    let PredictionStats { corrections, frames_reconciled, .. } = predictor.stats();
    let per_minute = corrections as f64 / seconds * 60.0;
    let per_frame = corrections as f64 / frames_reconciled.max(1) as f64;
    (per_minute, per_frame, predictor.ring().bytes())
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
