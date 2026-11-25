//! `record-stall` (the milestone 2 plan's verification row 21, decision 5.16, section 4's
//! table; D162): what the physics phase costs a frame's record beside it. Milestone 1's
//! composed tick, `lactose-sim`'s cost scene of 100,000 rows with 2,000 awake bodies on
//! Rapier beside it, steps on N frame workers (N the performance cores up to eight) while
//! the test thread records frame after frame of the binary's render side, `ClientRender`,
//! on `lactose-rhi-null`: each record one job from `Executor::run_job`, as the shown frame
//! runs its own, which opens a region, writes the binary's seven instances and calls
//! `RenderSide::draw` (the remote batch, `prepare`, the declaration, compile and record of
//! the minimal renderer's passes), timed from the job's start to `draw`'s return; the
//! submit follows on the test thread. It runs at a physics reservation of one frame worker
//! and of two, the solver pool sized as `lactose-app` sizes it, N less the reservation and
//! at least one, and records nothing: `RECORD_RESERVATION` stays as it is until the file is
//! read.
//!
//! The physics phase of each tick is the span from the first entry into its three systems'
//! nodes (`physics_api::sync`, `step` and `write_back`) to the last exit, taken from the
//! executor's spans. A record that overlaps it is *beside* the physics phase; one that
//! overlaps the tick and not the phase is *beside the tick*; and the same records made with
//! no tick running are *alone*. The file gives the median, the 99th percentile and the
//! worst of each, and the physics phase's median. 60 ticks at each reservation (6 in a
//! short run) after 10 of warm-up.
//!
//! Not measured here: on Windows, the time from each `block_in_place`'s end to the record's
//! next job on a worker that was parked (the `SetEvent` item of row 164's unmeasured list).
//! It needs a stamp where a frame-critical worker parks and wakes, inside `lactose-task`.

#![allow(dead_code, reason = "the scene as `lactose-sim`'s cost test has it")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lactose_app::{
    Draw, RECORD_RESERVATION, Region, RenderSide, TargetIndex, Workers, start_executor,
};
use lactose_app_client::{ClientRender, FIXED_EXTENT};
use lactose_asset_api::{AssetId, AssetRef};
use lactose_core::{NetId, PlayerId, Result, StableKey, ThreadClass, Tick};
use lactose_ecs::{
    ClaimedBy, Commands, Ctx, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
    RegionConfig, RegionId, RegionMode, RegionRes, Res, ResMut, StableKeyOf, SystemSet, Without,
    World, WorldKind, finalize, system,
};
use lactose_input_host::LatestSample;
use lactose_math::{ExactSum, Scalar, Vec3};
use lactose_physics_api::{Collider, PhysicsConfig, RigidBody, Shape, Transform};
use lactose_present_api::{ExtractPass, InstanceRow, RenderTransform};
use lactose_reflect::{component, resource};
use lactose_rhi_api::Capabilities;
use lactose_sim::{
    InputApplier, PlayerSample, Registration, Simulation, SimulationBuilder, Stage, TickDelta,
    TickInput, TypeTable, sets,
};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Metadata, Subscriber};

use crate::support::{bounded, lock, null_device, null_shaders};
use crate::timing::{enter, figure, measured_workers, median, micros, short, spread};

include!("../../sim/tests/cost/scene.rs");

const BODIES: u64 = 2_000;
/// The first key of the bodies: after the scene's rows and players.
const FIRST_BODY: u64 = 2_000_000;
/// The instances the binary's scene shows.
const INSTANCES: u64 = 7;
const WARM_UP: u32 = 10;

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

/// A floor 80 metres square with its top at zero, and on it [`BODIES`] dynamic bodies that
/// never sleep, as `lactose-app`'s composed tick places them.
fn floor_and_bodies(world: &mut World, region: RegionId, first: u64) {
    let floor = Collider::new(Shape::cuboid(Vec3::new(s(40.0), s(1.0), s(40.0))))
        .with_offset(Vec3::new(Scalar::ZERO, s(-1.0), Scalar::ZERO));
    world.spawn(key(first), region, (Transform::IDENTITY, floor)).unwrap();
    let mut body = RigidBody::dynamic();
    body.can_sleep = false;
    for n in 0..BODIES {
        let (layer, cell) = (n / 1_000, n % 1_000);
        let at = Vec3::new(
            s((cell % 50) as f64 - 25.0),
            s(0.5 + layer as f64),
            s((cell / 50) as f64 - 10.0),
        );
        let shape = match n % 3 {
            0 => Shape::sphere(s(0.4)),
            1 => Shape::cuboid(Vec3::new(s(0.4), s(0.4), s(0.4))),
            _ => Shape::capsule(s(0.2), s(0.3)),
        };
        let row = (Transform::at(at), body, Collider::new(shape));
        world.spawn(key(first + 1 + n), region, row).unwrap();
    }
}

/// The composed tick's world on `workers` frame workers at a physics reservation of
/// `reserved`, its solver pool as `lactose-app` sizes it.
fn composed(workers: u32, reserved: u32) -> Simulation {
    let threads = workers.saturating_sub(reserved).max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads as usize)
        .thread_name(|index| format!("solver-{index}"))
        .start_handler(|_| enter(ThreadClass::PhysicsSolver))
        .build()
        .unwrap();
    let backend = lactose_physics_rapier::backend(pool);
    let config = PhysicsConfig { reserved_workers: reserved, ..PhysicsConfig::default() };
    let mut types = types();
    lactose_physics_api::register_types(&mut types).unwrap();
    let mut simulation = builder(ROWS, types)
        .setup(move |world| lactose_physics_api::setup(world, backend, config))
        .systems(lactose_physics_api::register_systems)
        .systems(|registration| {
            registration.order(&lactose_physics_api::sets::WRITE_BACK, &POST);
        })
        .build()
        .unwrap();
    let world = simulation.world_mut();
    populate(world, ROWS);
    floor_and_bodies(world, RegionId(1), FIRST_BODY);
    simulation
}

// ---- The physics phase, from the executor's spans ----

/// The physics systems' nodes, as the executor names their spans.
const PHYSICS: [&str; 3] = ["physics_api::sync", "physics_api::step", "physics_api::write_back"];

/// The first entry into a physics node's span and the last exit since the last take: the
/// physics phase of one tick. A subscriber of its own: it sees the executor's node and range
/// spans of the three systems alone.
#[derive(Default)]
struct PhysicsPhase(Mutex<Option<(Instant, Instant)>>);

impl PhysicsPhase {
    fn take(&self) -> Option<(Instant, Instant)> {
        lock(&self.0).take()
    }
}

impl Subscriber for &'static PhysicsPhase {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) { Interest::always() } else { Interest::never() }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_span()
            && metadata.target() == "whisk"
            && metadata
                .module_path()
                .is_some_and(|path| path == "lactose_task::node" || path == "lactose_task::range")
            && PHYSICS.contains(&metadata.name())
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, _: &Id) {
        let now = Instant::now();
        let mut phase = lock(&self.0);
        *phase = Some(phase.map_or((now, now), |(first, last)| (first.min(now), last)));
    }

    fn exit(&self, _: &Id) {
        let now = Instant::now();
        let mut phase = lock(&self.0);
        *phase = Some(phase.map_or((now, now), |(first, last)| (first, last.max(now))));
    }
}

// ---- The record ----

/// The binary's render side on null, recorded one frame at a time.
struct Recorder {
    render: Arc<Mutex<ClientRender>>,
    rows: Arc<Vec<InstanceRow>>,
    frames: u32,
}

impl Recorder {
    fn new() -> Self {
        let (device, _, _) = null_device(Capabilities::LEAST);
        let input = Arc::new(LatestSample::new());
        let render =
            ClientRender::new(&device, &null_shaders(), input, None).expect("a render side");
        let rows = (1..=INSTANCES)
            .map(|n| InstanceRow {
                key: key(3_000_000 + n),
                mesh: AssetRef::new(AssetId::from_bits(n)),
                transform: RenderTransform::from_sim(&Transform::IDENTITY),
                no_interpolate: false,
            })
            .collect();
        Self { render: Arc::new(Mutex::new(render)), rows: Arc::new(rows), frames: 0 }
    }

    /// One frame: the job that records it, timed from its start to `draw`'s return, then
    /// its submit. Returns when the record began and ended.
    fn record(&mut self, executor: &Executor) -> (Instant, Instant) {
        let target = TargetIndex::new((self.frames % 2) as usize).expect("a target");
        let pass = ExtractPass {
            kind: WorldKind::Server,
            tick: Tick(self.frames),
            full: self.frames == 0,
        };
        self.frames += 1;
        let job = {
            let (render, rows) = (Arc::clone(&self.render), Arc::clone(&self.rows));
            executor.run_job(move |ctx| {
                let start = Instant::now();
                let mut render = lock(&render);
                assert_eq!(render.open(ctx).expect("a region"), Region::Ready, "null frees each");
                let mut sink = render.sink();
                sink.begin(pass);
                sink.write(&rows);
                sink.end();
                drop(sink);
                let draw = Draw { target, extent: FIXED_EXTENT, ready: None };
                render.draw(ctx, draw).expect("the frame records");
                (start, Instant::now())
            })
        };
        let span = bounded(executor, job, "a frame's record").expect("the job runs");
        lock(&self.render).submit().expect("the frame submits");
        span
    }
}

/// Every figure of one reservation, under `r<reserved>`.
fn beside(executor: &Executor, recorder: &mut Recorder, workers: u32, reserved: u32, ticks: u32) {
    let phase: &'static PhysicsPhase = physics_phase();
    let mut simulation = composed(workers, reserved);
    let (mut physics, mut steps) = (Vec::new(), Vec::new());
    let (mut by_physics, mut by_tick) = (Vec::new(), Vec::new());
    for tick in 0..WARM_UP + ticks {
        phase.take();
        let done = Arc::new(AtomicBool::new(false));
        let stepping = {
            let (executor, done) = (executor.clone(), Arc::clone(&done));
            std::thread::spawn(move || {
                let input = frame(simulation.tick().0 + 1);
                let start = Instant::now();
                let stepped =
                    bounded(&executor, simulation.step(&executor, input), "a composed tick");
                let span = (start, Instant::now());
                done.store(true, Ordering::Release);
                (stepped.expect("the tick runs"), span)
            })
        };
        let mut records = Vec::new();
        while !done.load(Ordering::Acquire) {
            records.push(recorder.record(executor));
        }
        let (stepped, (began, ended)) = stepping.join().expect("the stepping thread");
        simulation = stepped;
        let (first, last) = phase.take().expect("the tick ran its physics systems");
        if tick < WARM_UP {
            continue;
        }
        steps.push(ended - began);
        physics.push(last - first);
        for (start, end) in records {
            let time = end - start;
            if start < last && end > first {
                by_physics.push(time);
            } else if start < ended && end > began {
                by_tick.push(time);
            }
        }
    }
    let prefix = format!("r{reserved}");
    figure(&format!("{prefix}.physics_us"), micros(median(physics)));
    figure(&format!("{prefix}.step_us"), micros(median(steps)));
    figure(&format!("{prefix}.beside_physics.records"), by_physics.len());
    spread(&format!("{prefix}.beside_physics.record"), by_physics);
    figure(&format!("{prefix}.beside_tick.records"), by_tick.len());
    if !by_tick.is_empty() {
        spread(&format!("{prefix}.beside_tick.record"), by_tick);
    }
}

/// The subscriber, installed once per process (nextest runs each test in its own).
fn physics_phase() -> &'static PhysicsPhase {
    static PHASE: std::sync::OnceLock<&'static PhysicsPhase> = std::sync::OnceLock::new();
    PHASE.get_or_init(|| {
        let phase: &'static PhysicsPhase = Box::leak(Box::default());
        tracing::subscriber::set_global_default(phase).expect("no other subscriber");
        phase
    })
}

/// `cargo xtask measure record-stall` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: times are reported, not asserted"]
fn the_record_beside_the_physics_phase() {
    let ticks = if short() { 6 } else { 60 };
    let workers = measured_workers();
    let executor = start_executor(Workers { frame_workers: Some(workers), ..Workers::default() })
        .expect("an executor");
    let mut recorder = Recorder::new();
    for reserved in [1, 2] {
        beside(&executor, &mut recorder, workers, reserved, ticks);
    }
    let alone: Vec<Duration> = (0..600)
        .map(|_| {
            let (start, end) = recorder.record(&executor);
            end - start
        })
        .collect();
    figure("alone.records", alone.len());
    spread("alone.record", alone);
    figure("workers", workers);
    figure("run_job_width", executor.run_job_width());
    figure("record_reservation", RECORD_RESERVATION);
    figure("reservations", "1 and 2");
    figure("rows", ROWS + u64::from(PLAYERS));
    figure("bodies", BODIES);
    figure("instances", INSTANCES);
    figure("ticks", ticks);
    figure("warm_up_ticks", WARM_UP);
    drop(recorder);
    executor.shutdown();
}
