//! What the assembled engine costs at the milestone 1 scale (architecture section 22, plan
//! decision 5.1), with Rapier behind `physics-api` as this crate builds it: the physics
//! phase at 100k rows (criterion 4), the tick of `lactose-sim`'s cost scene with 2,000
//! bodies beside it, the full diff of a 100k reload (D124), and a body on every bone of
//! 200 characters. Each test is ignored and run in release by `cargo xtask measure`, which
//! writes the result file from the lines that start `measure: `; each asserts on counts and
//! prints its timings, never asserting one. One more prints the topology the executor
//! detected, for the result files' header.
//!
//! The scenes build their solver pools as `lactose-app` does, with `frame workers - 1`
//! threads and at least one (`PhysicsConfig::default()` reserves one worker): the physics
//! phase through `build_server` itself, the others by hand around a world of their own,
//! since their scenes are not a game.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

#[path = "../../anim/tests/common/mod.rs"]
mod anim_support;

#[path = "../../physics-api/tests/common/mod.rs"]
#[allow(
    clippy::duplicate_mod,
    reason = "its test backend is also `anim_support`'s; this file takes the region bundle and the reload frame"
)]
mod physics_support;

#[path = "../../sim/tests/cost/timing.rs"]
#[allow(
    unfulfilled_lint_expectations,
    reason = "the boundary lints the module's expectations name are not configured outside the boundary"
)]
mod timing;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_anim::{AnimRig, BoneLink, Condition};
use lactose_app::{Cooked, Game, Start, Workers, WorldConfig, build_server, start_executor};
use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, NetId, Result, StableKey};
use lactose_ecs::{RegionConfig, RegionId, RegionMode, World, WorldKind};
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{
    Collider, PhysicsBackend, PhysicsConfig, RigidBody, Shape, Transform, Velocity,
};
use lactose_sim::{Simulation, SimulationBuilder, TickInput, TypeTable};
use lactose_task::{Executor, Platform, StdPlatform};

// ---- What every measurement shares ----

/// Whether `cargo xtask measure --short` runs the test: fewer repetitions, the same figures.
fn short() -> bool {
    std::env::var_os("LACTOSE_MEASURE_SHORT").is_some()
}

/// One figure for `cargo xtask measure`, which reads the lines that start `measure: `.
fn figure(name: &str, value: impl std::fmt::Display) {
    println!("measure: {name} {value}");
}

fn micros(span: Duration) -> String {
    format!("{:.1}", span.as_secs_f64() * 1e6)
}

fn median(mut spans: Vec<Duration>) -> Duration {
    spans.sort();
    spans[spans.len() / 2]
}

fn executor(workers: Option<u32>) -> Executor {
    start_executor(Workers { frame_workers: workers, ..Workers::default() }).unwrap()
}

/// N of plan decision 5.1: the frame workers a default executor starts, one per performance
/// core it counts (architecture section 5.1), at most eight.
fn performance_workers() -> u32 {
    let executor = executor(None);
    let workers = executor.frame_worker_count().min(8);
    executor.shutdown();
    workers
}

/// The worker counts a scene runs on: one, and N when that is more.
fn worker_counts() -> Vec<u32> {
    let n = performance_workers();
    if n > 1 { vec![1, n] } else { vec![1] }
}

/// The solver pool `lactose-app` gives a world on `frame_workers` frame workers, with the
/// default reservation of one, and the backend over it.
fn rapier(frame_workers: u32) -> Box<dyn PhysicsBackend> {
    let threads = frame_workers.saturating_sub(PhysicsConfig::default().reserved_workers).max(1);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads as usize).build().unwrap();
    lactose_physics_rapier::backend(pool)
}

/// The `casein` spans, installed once per process (nextest runs each test in its own).
fn spans() -> &'static timing::Spans {
    let spans: &'static timing::Spans = Box::leak(Box::default());
    tracing::subscriber::set_global_default(spans).unwrap();
    spans
}

const SYNC: &str = "system physics_api::sync";
const STEP: &str = "system physics_api::step";
const WRITE_BACK: &str = "system physics_api::write_back";

/// The time of the three physics systems in one tick's spans.
fn physics_of(phases: &BTreeMap<String, Duration>) -> Duration {
    [SYNC, STEP, WRITE_BACK].iter().filter_map(|label| phases.get(*label)).sum()
}

/// The topology the executor detects and the frame workers a default executor starts:
/// what `cargo xtask measure` writes into every result file's header.
#[test]
#[ignore = "run by `cargo xtask measure` for its result files' header"]
fn the_topology_the_executor_detects() {
    let topology = StdPlatform.topology();
    figure("topology.performance", topology.performance);
    figure("topology.efficiency", topology.efficiency);
    figure("frame_workers", performance_workers());
}

// ---- The physics rows ----

const BODIES: u64 = 2_000;
const STATICS: u64 = 50_000;
/// The rows with a transform alone that bring the physics phase's scene to 100,000, with
/// the floor, the bodies and the statics.
const PLAIN: u64 = 100_000 - 1 - BODIES - STATICS;

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn net(n: u64) -> StableKey {
    StableKey::Net(NetId::cooked(n))
}

/// A floor 80 metres square with its top at zero, and on it [`BODIES`] dynamic bodies that
/// never sleep, in two layers of a 50 by 20 grid a metre apart, spheres, boxes and capsules
/// in turn, each touching the one below it: keyed from `first`.
fn floor_and_bodies(world: &mut World, region: RegionId, first: u64) {
    let floor = Collider::new(Shape::cuboid(Vec3::new(s(40.0), s(1.0), s(40.0))))
        .with_offset(Vec3::new(Scalar::ZERO, s(-1.0), Scalar::ZERO));
    world.spawn(net(first), region, (Transform::IDENTITY, floor)).unwrap();
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
        world.spawn(net(first + 1 + n), region, row).unwrap();
    }
}

/// [`floor_and_bodies`], then [`STATICS`] fixed boxes two metres apart away from the floor
/// and [`PLAIN`] rows with a transform alone: 100,000 rows keyed from `first`.
fn physics_rows(world: &mut World, region: RegionId, first: u64) {
    floor_and_bodies(world, region, first);
    let cube = Collider::new(Shape::cuboid(Vec3::new(s(0.5), s(0.5), s(0.5))));
    let statics = first + 1 + BODIES;
    for n in 0..STATICS {
        let at =
            Vec3::new(s(100.0 + 2.0 * (n % 250) as f64), Scalar::ZERO, s(2.0 * (n / 250) as f64));
        world.spawn(net(statics + n), region, (Transform::at(at), cube)).unwrap();
    }
    let plain = statics + STATICS;
    for n in 0..PLAIN {
        world.spawn(net(plain + n), region, (Transform::IDENTITY,)).unwrap();
    }
}

// ---- The physics phase ----

/// Content with nothing cooked: the physics phase's region is created in the world.
struct Nothing;

impl Cooked for Nothing {
    fn baseline(&self, _: u64) -> Result<Arc<Baseline>> {
        Err(Error::new(ErrorKind::NotFound, "nothing is cooked"))
    }

    fn table(&self, _: RegionId) -> Option<VersionTable> {
        None
    }

    fn pin(&self, _: &VersionTable) -> Result<PinnedTable> {
        Err(Error::new(ErrorKind::NotFound, "nothing is published"))
    }
}

static EMPTY: Game = Game { name: "cost", crates: &[], order: None, sessions: None };

/// Criterion 4 (architecture section 22, plan decision 5.1): the physics phase of a server
/// world `build_server` assembles on the default executor, its solver pool the frame
/// workers less one, over 100,000 rows: [`BODIES`] awake bodies on a floor, [`STATICS`]
/// fixed boxes and [`PLAIN`] rows with a transform alone. The phase is the sync, the step
/// and the write-back, each from its `casein` span; each figure is the median over 60
/// ticks (10 in a short run) after 30 of warm-up, and the whole server step is printed
/// beside it. `cargo xtask measure physics-phase` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
fn cost_of_the_physics_phase_at_100k_rows() {
    let (warm, ticks) = if short() { (5, 10) } else { (30, 60) };
    let spans = spans();
    let executor = executor(None);
    let config = WorldConfig::headless();
    let solver = executor.frame_worker_count().saturating_sub(config.physics.reserved_workers);
    let mut server =
        build_server(&executor, &EMPTY, config, Start::Fresh, Arc::new(Nothing)).unwrap();
    let world = server.simulation_mut().world_mut();
    let region = RegionId(1);
    world.create_region(RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated }).unwrap();
    physics_rows(world, region, 1);
    assert_eq!(world.entities_in(region).count(), 100_000);

    let (mut sync, mut step, mut write_back, mut phase, mut whole) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for tick in 0..warm + ticks {
        spans.take();
        let start = Instant::now();
        let (next, _) = executor.block_on(server.step(&executor, TickInput::default())).unwrap();
        let elapsed = start.elapsed();
        server = next;
        let phases = spans.take();
        if tick < warm {
            continue;
        }
        whole.push(elapsed);
        sync.push(phases[SYNC]);
        step.push(phases[STEP]);
        write_back.push(phases[WRITE_BACK]);
        phase.push(physics_of(&phases));
    }
    let world = server.simulation_mut().world_mut();
    let bodies = world.query::<&RigidBody>().count();
    let moving = world.query::<&Velocity>().count();
    assert_eq!(bodies, BODIES as usize);
    assert_eq!(moving, BODIES as usize, "the write-back gave every body its velocity");

    figure("rows", 100_000);
    figure("bodies", BODIES);
    figure("statics", STATICS + 1);
    figure("plain_rows", PLAIN);
    figure("frame_workers", executor.frame_worker_count());
    figure("solver_threads", solver.max(1));
    figure("ticks", ticks);
    figure("warm_up_ticks", warm);
    figure("sync_us", micros(median(sync)));
    figure("step_us", micros(median(step)));
    figure("write_back_us", micros(median(write_back)));
    figure("phase_us", micros(median(phase)));
    figure("server_step_us", micros(median(whole)));
    server.shutdown();
    executor.shutdown();
}

// ---- The composed tick ----

/// `lactose-sim`'s cost scene, included as its own test includes it, with 2,000 bodies on
/// a floor in its first region.
mod composed {
    #![allow(dead_code, reason = "the scene as `lactose-sim`'s cost test has it")]

    use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
    use lactose_ecs::{
        ClaimedBy, Commands, Ctx, Event, EventWriter, Events, Executor, LayoutConfig, Mut,
        RegionConfig, RegionId, RegionMode, RegionRes, Res, ResMut, StableKeyOf, SystemSet,
        Without, World, WorldKind, finalize, system,
    };
    use lactose_math::{ExactSum, Scalar};
    use lactose_physics_api::PhysicsConfig;
    use lactose_reflect::{component, resource};
    use lactose_sim::{
        InputApplier, PlayerSample, Registration, Simulation, SimulationBuilder, Stage, TickDelta,
        TickInput, TypeTable, sets,
    };

    use super::{figure, floor_and_bodies, median, micros, physics_of, rapier, spans};

    include!("../../sim/tests/cost/scene.rs");

    /// The first key of the bodies: after the scene's rows and players.
    const FIRST_BODY: u64 = 2_000_000;

    /// The scene's world on `workers` frame workers with physics beside it.
    fn world(workers: u32) -> Simulation {
        let mut types = types();
        lactose_physics_api::register_types(&mut types).unwrap();
        let backend = rapier(workers);
        let mut simulation = builder(ROWS, types)
            .setup(move |world| {
                lactose_physics_api::setup(world, backend, PhysicsConfig::default())
            })
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

    /// The tick of criterion 1's scene (`crates/sim/tests/cost.rs`) with 2,000 bodies on a
    /// floor beside it, as one world steps them: on one frame worker and on N, the
    /// performance cores up to eight, each with the solver pool `lactose-app` would give
    /// it; the median of 60 ticks (6 in a short run) after 10 of warm-up, the physics
    /// systems' share from their spans, and criterion 1's S and projection over the
    /// composed tick. `cargo xtask measure composed-tick` runs it.
    #[test]
    #[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
    fn cost_of_the_composed_tick() {
        const WARM: u32 = 10;
        let ticks: u32 = if super::short() { 6 } else { 60 };
        let spans = spans();
        let counts = super::worker_counts();
        let mut medians = Vec::new();
        for &workers in &counts {
            let executor = super::executor(Some(workers));
            let mut simulation = world(workers);
            let (mut steps, mut physics) = (Vec::new(), Vec::new());
            for tick in 0..WARM + ticks {
                spans.take();
                let start = std::time::Instant::now();
                simulation = step(&executor, simulation);
                let elapsed = start.elapsed();
                let phases = spans.take();
                if tick >= WARM {
                    steps.push(elapsed);
                    physics.push(physics_of(&phases));
                }
            }
            let world = simulation.world_mut();
            assert_eq!(world.query::<&lactose_physics_api::RigidBody>().count(), 2_000);
            assert!(world.query::<&Position>().count() >= ROWS as usize);
            let step = median(steps);
            figure(&format!("w{workers}.step_us"), micros(step));
            figure(&format!("w{workers}.physics_us"), micros(median(physics)));
            medians.push(step.as_secs_f64() * 1e6);
            drop(simulation);
            executor.shutdown();
        }
        let workers: Vec<String> = counts.iter().map(u32::to_string).collect();
        figure("workers", workers.join(" and "));
        figure("rows", ROWS + u64::from(PLAYERS));
        figure("bodies", 2_000);
        figure("ticks", ticks);
        figure("warm_up_ticks", WARM);
        let n = *counts.last().unwrap();
        figure("n", n);
        figure("t1_us", format!("{:.1}", medians[0]));
        if let [t1, tn] = medians[..] {
            let nf = f64::from(n);
            let serial = (nf * tn - t1) / (nf - 1.0);
            figure("tn_us", format!("{tn:.1}"));
            figure("s_us", format!("{serial:.1}"));
            figure("projection_us", format!("{:.1}", serial + (t1 - serial) / 8.0));
        }
    }
}

// ---- The reload ----

/// The D124 row: a 100k-row world with Rapier reloaded in place, its region unloaded and
/// loaded from its own bundle in one frame, so the next sync diffs every row against the
/// carried bookkeeping. The step of the reload frame and its sync, the median of 5 reloads
/// (1 in a short run) each 10 ticks after the last, beside the median of the ticks
/// between them; the bundle is built outside the timing. On the default executor, its
/// solver pool as `lactose-app` would give it. `cargo xtask measure reload-100k` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
fn cost_of_a_100k_reload() {
    const BETWEEN: u32 = 10;
    let reloads = if short() { 1 } else { 5 };
    let spans = spans();
    let executor = executor(None);
    let workers = executor.frame_worker_count();
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).unwrap();
    let backend = rapier(workers);
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
        .setup(move |world| lactose_physics_api::setup(world, backend, PhysicsConfig::default()))
        .systems(lactose_physics_api::register_systems)
        .build()
        .unwrap();
    let region = physics_support::REGION;
    let world = simulation.world_mut();
    world.create_region(RegionConfig { id: region, seed: 1, mode: RegionMode::Simulated }).unwrap();
    physics_rows(world, region, 1);
    let step = |simulation: Simulation, input: TickInput| {
        let start = Instant::now();
        let simulation = executor.block_on(simulation.step(&executor, input)).unwrap();
        (simulation, start.elapsed())
    };
    let (mut normal, mut normal_sync, mut reload, mut reload_sync) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for _ in 0..reloads {
        for _ in 0..BETWEEN {
            spans.take();
            let (next, elapsed) = step(simulation, TickInput::default());
            simulation = next;
            normal.push(elapsed);
            normal_sync.push(spans.take()[SYNC]);
        }
        let bundle = physics_support::bundle(simulation.world(), region);
        spans.take();
        let (next, elapsed) =
            step(simulation, physics_support::frame([region], [(region, bundle)]));
        simulation = next;
        reload.push(elapsed);
        reload_sync.push(spans.take()[SYNC]);
        let world = simulation.world_mut();
        assert_eq!(world.entities_in(region).count(), 100_000, "the region reloaded whole");
        assert_eq!(world.query::<&RigidBody>().count(), BODIES as usize);
    }
    figure("rows", 100_000);
    figure("bodies", BODIES);
    figure("statics", STATICS + 1);
    figure("frame_workers", workers);
    figure("reloads", reloads);
    figure("reload.step_us", micros(median(reload)));
    figure("reload.sync_us", micros(median(reload_sync)));
    figure("tick.step_us", micros(median(normal)));
    figure("tick.sync_us", micros(median(normal_sync)));
    drop(simulation);
    executor.shutdown();
}

// ---- Bone bodies ----

/// The characters of the bone bodies' scene.
const CHARACTERS: u64 = 200;
const BONES: u8 = 16;

fn rapier_for(workers: u32) -> Box<dyn PhysicsBackend> {
    rapier(workers)
}

/// `lactose-anim`'s cost scene (c), with a body on every bone: walk and run, a fade from
/// one to the other that half the characters are given and never finish.
fn bone_library() -> anim_support::Library {
    use anim_support::{MACHINE, SKELETON, chain, clip, clip_ref, state, transition};
    use lactose_anim::StateMachine;
    let (walk, run) = (clip_ref("cost/walk"), clip_ref("cost/run"));
    let mut fade = transition(Some(0), 1, &[Condition::Trigger(0)]);
    fade.blend_ticks = u16::MAX - 1;
    let machine = StateMachine {
        states: vec![state("walk", walk, 1.0), state("run", run, 1.3)],
        transitions: vec![fade],
        initial: 0,
    };
    anim_support::Library::new()
        .with(SKELETON, chain(usize::from(BONES), |_| true))
        .with(MACHINE, machine)
        .with(walk, clip(BONES, 60, 60, true, 0.0))
        .with(run, clip(BONES, 60, 45, true, 0.7))
}

/// The bone bodies' scene: 200 characters four metres apart on a 20 by 10 grid, half of
/// them mid fade, over Rapier on `workers`, with a ragdoll spawned on every one at tick 2
/// when `bones`.
fn bone_scene(workers: u32, bones: bool) -> anim_support::Fixture {
    use anim_support::{MACHINE, Options, RagdollAction, SKELETON, key};
    let mut options = Options::new(WorldKind::Server);
    options.physics = true;
    options.backend = Some(rapier_for);
    options.workers = workers;
    let mut fixture = options.build_on(executor(Some(workers)));
    fixture.assets(bone_library().pin());
    for n in 1..=CHARACTERS {
        let at = Vec3::new(s(4.0 * (n % 20) as f64), Scalar::ZERO, s(4.0 * (n / 20) as f64));
        fixture.spawn_at(key(n), AnimRig::new(MACHINE, SKELETON), Transform::at(at));
        if n.is_multiple_of(2) {
            fixture.set_params(key(n), |params| params.trigger(0));
        }
    }
    if bones {
        let script = fixture.world_mut().service_mut::<anim_support::Ragdoll>().unwrap();
        script.actions.extend((1..=CHARACTERS).map(|n| (2, key(n), RagdollAction::Spawn)));
    }
    fixture
}

/// The bone bodies on Rapier (`lactose-anim`'s cost test, (c), there over the physics test
/// backend): 200 characters with a body on every one of their sixteen bones, against the
/// same characters without, on one frame worker and on N; the median step of 300 ticks (30
/// in a short run) after 50 of warm-up. `cargo xtask measure bone-bodies` runs it.
#[test]
#[ignore = "a measurement, run by `cargo xtask measure`: timings are reported, not asserted"]
fn cost_of_bone_bodies_on_rapier() {
    const WARM: u32 = 50;
    let ticks = if short() { 30 } else { 300 };
    let spans = |fixture: &mut anim_support::Fixture| {
        fixture.run(WARM);
        let timed = |fixture: &mut anim_support::Fixture| {
            let start = Instant::now();
            fixture.step();
            start.elapsed()
        };
        median((0..ticks).map(|_| timed(fixture)).collect())
    };
    figure("characters", CHARACTERS);
    figure("bones", BONES);
    figure("ticks", ticks);
    figure("warm_up_ticks", WARM);
    let counts = worker_counts();
    let workers: Vec<String> = counts.iter().map(u32::to_string).collect();
    figure("workers", workers.join(" and "));
    for workers in counts {
        let mut with = bone_scene(workers, true);
        let with_span = spans(&mut with);
        let rows = with.world().columns::<BoneLink>().map(|c| c.keys().len()).sum::<usize>();
        assert_eq!(rows, (CHARACTERS * u64::from(BONES)) as usize);
        let bodies = with.world_mut().query::<&RigidBody>().count();
        assert_eq!(bodies, rows, "a body on every bone");
        let mut without = bone_scene(workers, false);
        let without_span = spans(&mut without);
        figure(&format!("w{workers}.with_us"), micros(with_span));
        figure(&format!("w{workers}.without_us"), micros(without_span));
        figure(&format!("w{workers}.bodies_us"), micros(with_span.saturating_sub(without_span)));
    }
}
