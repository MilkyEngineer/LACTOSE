//! Compile-only consumer sketches against the facade: what `app`, a game crate, `input`,
//! `net`, `predict` and `persist` write, type-checked and never run (every deciding body
//! of the crate is `todo!()`). They pin the surface the README promises: a change that
//! breaks one is a signature change to list. Adapted from the design review's probes.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::sync::Arc;

use lactose_core::{NetId, PlayerId, PredictionKey, Result, StableKey, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Executor, Job, Mut, RegionId, RegionMode, Res, ResMut, SystemSet,
    World, WorldKind, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::{component, resource};
use lactose_sim::{
    CurrentTick, InputApplier, Pass, PlayerSample, RegionBundle, RegionExport, Registration,
    SavePoint, SessionApplier, SessionEvent, Simulation, SimulationBuilder, Stage, StagedState,
    StreamingAnchor, TICK_RATE_HZ, TickDelta, TickInput, TimeScale, TypeTable, sets,
};
use lactose_task::{TickPacer, WorkerClass};

// ---- 1. A game crate's register_types / register_systems pair ----

#[component(key = "sketch::velocity", persistent, replicated)]
pub struct Velocity {
    #[field(id = 1)]
    pub x: Scalar,
    #[field(id = 2)]
    pub y: Scalar,
}

#[component(key = "sketch::mass", persistent)]
pub struct Mass(#[field(id = 1)] pub Scalar);

/// Configuration: a world resource, so `derived`.
#[resource(key = "sketch::gravity", derived)]
pub struct Gravity(#[field(id = 1)] pub Scalar);

/// What a reduction writes: a world resource, so `derived` (it is recomputed every tick).
#[resource(key = "sketch::total_mass", derived)]
pub struct TotalMass(#[field(id = 1)] pub Scalar);

pub static MOVEMENT: SystemSet = SystemSet::new("sketch::movement");

#[system(name = "sketch::apply_gravity", worlds = [server, prediction], in_set = MOVEMENT)]
fn apply_gravity(dt: Res<TickDelta>, g: Res<Gravity>, vel: Mut<Velocity>, mass: &Mass) {
    vel.y -= g.0 * dt.0;
}

#[system(name = "sketch::total_mass", worlds = [server], reduce = ExactSum<Scalar>)]
fn total_mass(mass: &Mass) -> Scalar {
    mass.0
}

#[finalize(total_mass)]
fn store_total(sum: Scalar, out: ResMut<TotalMass>) {
    out.0 = sum;
}

/// A once-per-tick system reading the tick without `Ctx`; it does not read `TimeScale`.
#[system(name = "sketch::tick_reader", worlds = [server])]
fn tick_reader(now: Res<CurrentTick>) {
    let _: Tick = now.get();
}

/// A per-region system.
#[system(name = "sketch::per_region", worlds = [server, prediction])]
fn per_region(ctx: Ctx, cmd: Commands) {}

/// A system that must run after physics: placed in `Stage::Post`.
#[system(name = "sketch::after_physics", worlds = [server, prediction])]
fn after_physics(vel: &Velocity) {}

pub fn register_types(types: &mut TypeTable) -> Result<()> {
    types.register::<Velocity>()?;
    types.register::<Mass>()?;
    types.register::<Gravity>()?;
    types.register::<TotalMass>()?;
    types.register::<InputSample>()?;
    Ok(())
}

/// What a setup function inserts: every resource a registered system names.
pub fn setup(world: &mut World) -> Result<()> {
    world.insert_resource(Gravity(Scalar::from_i32(-10)));
    world.insert_resource(TotalMass(Scalar::ZERO));
    Ok(())
}

pub fn register_systems(registration: &mut Registration) {
    registration
        .stage(Stage::Simulation)
        .sets(&[&MOVEMENT])
        .add(&APPLY_GRAVITY)
        .add(&TOTAL_MASS)
        .add(&TICK_READER)
        .add(&PER_REGION);
    registration.stage(Stage::Post).add(&AFTER_PHYSICS);
}

/// How `app` builds: types, setup functions, registration functions, hooks.
fn app_build(kind: WorldKind) -> Result<Simulation> {
    let mut types = TypeTable::new();
    register_types(&mut types)?;
    SimulationBuilder::new(kind, types)
        .setup(setup)
        .systems(register_systems)
        .systems(register_net)
        .inputs(Box::new(Applier))
        .sessions(Box::new(Sessions))
        .staged_state(Box::new(Staged { region: RegionId(1) }))
        .log_retention(SimulationBuilder::DEFAULT_LOG_RETENTION)
        .build()
}

// ---- 2. An InputApplier over a #[component] with CheckedBitPattern ----

#[component(key = "sketch::input_sample", derived)]
pub struct InputSample {
    #[field(id = 1)]
    pub buttons: u32,
    #[field(id = 2)]
    pub yaw: i32,
}

/// The player's own entity: the claimed row that is also an anchor.
fn player_entity(world: &mut World, player: PlayerId) -> Option<StableKey> {
    let mut claimed = Vec::new();
    world.query::<&ClaimedBy>().for_each(|key, claim| {
        if claim.player == player {
            claimed.push(key);
        }
    });
    claimed.into_iter().find(|&key| world.get::<StreamingAnchor>(key).is_some())
}

struct Applier;

impl InputApplier for Applier {
    fn apply(&mut self, world: &mut World, player: PlayerId, sample: &PlayerSample) -> Result<()> {
        let sample: InputSample = lactose_reflect::decode(&sample.sample)?;
        let key = player_entity(world, player).ok_or_else(|| {
            lactose_core::Error::new(lactose_core::ErrorKind::NotFound, "no entity")
        })?;
        world.insert(key, sample)
    }
}

// ---- 3. A SessionApplier spawning the player's entity with its claim and anchor ----

struct Sessions;

impl SessionApplier for Sessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::Join { region } => {
                let key = StableKey::Net(world.allocate_net_id()?);
                world.spawn(
                    key,
                    region,
                    (Mass(Scalar::ONE), ClaimedBy { player }, StreamingAnchor { priority: 0 }),
                )
            }
            SessionEvent::Leave => match player_entity(world, player) {
                Some(key) => world.despawn(key),
                None => Ok(()),
            },
        }
    }
}

// ---- 4. A StagedState spawning with a Net key and binding ----

struct Staged {
    region: RegionId,
}

impl StagedState for Staged {
    fn apply(&mut self, world: &mut World, tick: Tick) -> Result<()> {
        let net = NetId::from_bits(7).unwrap();
        world.spawn(StableKey::Net(net), self.region, (Mass(Scalar::ONE),))?;
        let predicted = PredictionKey { player: PlayerId(1), input_tick: tick, ordinal: 3 };
        world.bind(predicted, net)?;
        if let Some(mut vel) = world.get_mut::<Velocity>(StableKey::Net(net)) {
            vel.set_if_neq(Velocity { x: Scalar::ZERO, y: Scalar::ONE });
        }
        Ok(())
    }
}

// ---- 5. An exclusive system at the sync point, after sim's, and a Pass::Gather system ----

pub static AFTER_FRAME: SystemSet = SystemSet::new("sketch::after_frame").after(&[&sets::INPUT]);

/// The input component's own writer declaration for transient readers.
#[system(name = "sketch::input_writer", worlds = [server, prediction], exclusive, in_set = AFTER_FRAME, writes = [InputSample])]
fn input_writer(world: &mut World) {}

#[system(name = "sketch::gather", worlds = [server], exclusive)]
fn gather(world: &mut World, job: Job) {
    let world: &World = world;
    job.context().scope(|scope| {
        for column in world.columns::<Velocity>() {
            scope.fork(move |_child| {
                let _ = column.keys().len();
            });
        }
    });
}

pub fn register_net(registration: &mut Registration) {
    registration.stage(Stage::Input).sets(&[&AFTER_FRAME]).add(&INPUT_WRITER);
    registration.pass(Pass::Gather).add(&GATHER);
}

// ---- 6. The timer root sketch against TickPacer's real signatures ----

/// `app`'s task: the one place a clock is read and the executor's timers are used.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "the timer root is `app`'s, outside the boundary; this sketch pins that it \
              compiles against `step` and `run_passes`"
)]
async fn timer_root(
    executor: Executor,
    mut simulation: Simulation,
    mut next_frame: impl FnMut() -> TickInput + Send,
    mut time_scale: impl FnMut() -> TimeScale + Send,
) -> Result<()> {
    use std::time::{Duration, Instant};
    let period = Duration::from_secs(1) / TICK_RATE_HZ;
    let mut pacer = TickPacer::new(Instant::now(), period, 4);
    let resimulated_this_frame = 0;
    loop {
        let budget = pacer.poll(Instant::now(), resimulated_this_frame);
        for _ in 0..budget.ticks {
            simulation = simulation.step(&executor, next_frame()).await?;
        }
        simulation = simulation.run_passes(&executor).await?;
        if budget.behind { /* the ladder: app's */ }
        // The pacer's scale is app's own copy, never read back from the world.
        pacer.set_time_scale(f64::from(time_scale().thousandths) / 1000.0);
        executor.sleep_until(budget.next_deadline).await?;
    }
}

// ---- 7. Runs driven by block_on and from inside a 'static spawned task ----

fn drive_block_on(executor: &Executor, simulation: Simulation) -> Result<Simulation> {
    let simulation = executor.block_on(simulation.step(executor, TickInput::default()))?;
    executor.block_on(simulation.run_passes(executor)).map_err(Into::into)
}

/// A kind check or a prepare failure hands the simulation back inside the error.
fn keep_on_failure(executor: &Executor, simulation: Simulation) -> Option<Simulation> {
    match executor.block_on(simulation.save_point(executor)) {
        Ok((simulation, save)) => Some(simulation),
        Err(failure) => failure.simulation,
    }
}

#[expect(clippy::disallowed_methods, reason = "app spawns the timer root; this pins that it can")]
fn drive_spawned(executor: &Executor, simulation: Simulation) {
    let owned = executor.clone();
    let task = executor.spawn(WorkerClass::FrameCritical, async move {
        let mut simulation = simulation;
        for _ in 0..3 {
            simulation = simulation.step(&owned, TickInput::default()).await?;
        }
        let (simulation, save) = simulation.save_point(&owned).await?;
        Ok::<_, lactose_core::Error>(simulation.tick())
    });
    task.detach();
}

// ---- 8. Replacing a hook after build ----

fn replace_hook(simulation: &mut Simulation) {
    simulation.world_mut().insert_service::<dyn InputApplier>(Box::new(Applier));
}

// ---- 9. persist: a load from a save point, region by region through the frame ----

fn load_from_save(save: &SavePoint, baseline: Arc<lactose_bundle::Baseline>) -> TickInput {
    let mut frame = TickInput::default();
    for RegionExport { region, rows } in &save.regions {
        frame.loaded.insert(
            *region,
            RegionBundle {
                seed: rows.seed,
                mode: RegionMode::Frozen,
                baseline: baseline.clone(),
                overlay: None,
                resources: rows.resources.clone(),
                transient_resources: rows.transient_resources.clone(),
                assets: None,
            },
        );
    }
    frame
}

fn rebuild(save: &SavePoint, types: TypeTable) -> Result<Simulation> {
    SimulationBuilder::new(WorldKind::Server, types)
        .tick(save.tick)
        .next_net_id(save.next_net_id)
        .build()
}

// ---- 10. The save copy's shape: one child per region, results into disjoint cells ----

#[system(name = "sketch::save_copy_shape", worlds = [server], exclusive)]
fn save_copy_shape(world: &mut World, job: Job) {
    let world: &World = world;
    let regions: Vec<RegionId> = world.regions().collect();
    let mut cells: Vec<Option<Result<lactose_ecs::RegionRows>>> =
        regions.iter().map(|_| None).collect();
    job.context().scope(|scope| {
        for (cell, &region) in cells.iter_mut().zip(&regions) {
            scope.fork(move |_child| *cell = Some(world.export_region(region)));
        }
    });
}
