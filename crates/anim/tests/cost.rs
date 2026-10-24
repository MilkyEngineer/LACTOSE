//! The budget of the sim skeleton (`README.md`, "Proof for milestone 1", the budget row;
//! `docs/verification.md`; architecture section 22's 0.5 ms for 200 characters): 200
//! characters without `Bones`, each a sixteen-bone skeleton and a 60-sample looping clip,
//! half of them mid cross-fade. (a) `SimPose::evaluate` over the 200 for 1,000 ticks; (b)
//! `Simulation::step` of the world holding them against the same world whose characters
//! lack `SimPose`, on one and four workers; (c) the same 200 with `Bones` and a body on
//! every bone, against the same scene without them, over the physics test backend with its
//! contact search off and with no physics at all. Asserts
//! on counts (every pose evaluated, half the characters blending, the bone rows there);
//! timings are printed, never asserted: run it by hand in release on an idle machine,
//! `cargo test -p lactose-anim --release --test cost -- --ignored --nocapture`, with
//! `uptime` before and after.

#![forbid(unsafe_code)]

mod common;

use std::hint::black_box;
use std::time::Duration;

use common::{
    Fixture, Library, MACHINE, Options, Ragdoll, RagdollAction, SKELETON, chain, clip, clip_ref,
    executor, key, state, transition,
};
use lactose_anim::{
    AnimParams, AnimRig, AnimState, BoneLink, Clip, Condition, Outgoing, Phase, RootMotion,
    SimPose, StateMachine,
};
use lactose_asset_api::AssetRef;
use lactose_ecs::{Executor, RegionConfig, RegionMode, WorldKind};
use lactose_math::Scalar;
use lactose_physics_api::Transform;
use lactose_sim::{Simulation, SimulationBuilder, TickInput, TypeTable};

const CHARACTERS: u64 = 200;
const BONES: u8 = 16;
const FRAMES: u32 = 60;
const WARM_UP: u32 = 50;
const TICKS: u32 = 1_000;

const WALK: AssetRef<Clip> = clip_ref("cost/walk");
const RUN: AssetRef<Clip> = clip_ref("cost/run");
const FADE: u8 = 0;

/// Walk and run, sixty frames each; a trigger fades walk into run over a blend longer
/// than the run, so a character given it stays mid cross-fade.
fn library(bodies: bool) -> Library {
    let mut fade = transition(Some(0), 1, &[Condition::Trigger(FADE)]);
    fade.blend_ticks = u16::MAX - 1;
    let machine = StateMachine {
        states: vec![state("walk", WALK, 1.0), state("run", RUN, 1.3)],
        transitions: vec![fade],
        initial: 0,
    };
    Library::new()
        .with(SKELETON, chain(usize::from(BONES), |_| bodies))
        .with(MACHINE, machine)
        .with(WALK, clip(BONES, FRAMES, 60, true, 0.0))
        .with(RUN, clip(BONES, FRAMES, 45, true, 0.7))
}

#[expect(
    clippy::disallowed_types,
    reason = "a measurement outside the step: printed, never asserted"
)]
fn timed(work: impl FnOnce()) -> Duration {
    let start = std::time::Instant::now();
    work();
    start.elapsed()
}

fn median(mut spans: Vec<Duration>) -> Duration {
    spans.sort();
    spans[spans.len() / 2]
}

fn micros(span: Duration) -> f64 {
    span.as_secs_f64() * 1e6
}

/// Half the characters are given the fade's trigger at spawn.
fn params(n: u64) -> AnimParams {
    let mut params = AnimParams::ZERO;
    if n.is_multiple_of(2) {
        params.trigger(FADE);
    }
    params
}

/// The world of (b): this crate's systems alone, the 200 characters, with or without
/// `SimPose`.
fn step_world(with_pose: bool) -> Simulation {
    let mut types = TypeTable::new();
    lactose_physics_api::register_types(&mut types).unwrap();
    lactose_anim::register_types(&mut types).unwrap();
    let mut simulation = SimulationBuilder::new(WorldKind::Server, types)
        .systems(lactose_anim::register_systems)
        .build()
        .unwrap();
    let world = simulation.world_mut();
    world
        .create_region(RegionConfig { id: common::REGION, seed: 1, mode: RegionMode::Simulated })
        .unwrap();
    world.set_region_assets(common::REGION, library(false).pin()).unwrap();
    for n in 1..=CHARACTERS {
        let row = (
            Transform::IDENTITY,
            AnimRig::new(MACHINE, SKELETON),
            AnimState::UNSET,
            params(n),
            RootMotion::ZERO,
        );
        if with_pose {
            world
                .spawn(key(n), common::REGION, (row.0, row.1, row.2, row.3, row.4, SimPose::EMPTY))
                .unwrap();
        } else {
            world.spawn(key(n), common::REGION, row).unwrap();
        }
    }
    simulation
}

fn step(executor: &Executor, simulation: Simulation) -> Simulation {
    executor.block_on(simulation.step(executor, TickInput::default())).unwrap()
}

/// The per-tick spans of `TICKS` steps after the warm-up, and the world.
fn step_spans(executor: &Executor, mut simulation: Simulation) -> (Vec<Duration>, Simulation) {
    for _ in 0..WARM_UP {
        simulation = step(executor, simulation);
    }
    let mut spans = Vec::with_capacity(TICKS as usize);
    for _ in 0..TICKS {
        let mut next = None;
        spans.push(timed(|| next = Some(step(executor, simulation))));
        simulation = next.unwrap();
    }
    (spans, simulation)
}

fn blending(world: &lactose_ecs::World) -> usize {
    (1..=CHARACTERS).filter(|&n| world.get::<AnimState>(key(n)).unwrap().blend().is_some()).count()
}

#[test]
#[ignore = "a measurement, run by hand in release"]
fn the_sim_skeleton_budget() {
    let authored = library(false);
    let table = authored.pin();
    let skeleton = table.get(SKELETON).unwrap();
    let walk = table.get(WALK).unwrap();
    let run = table.get(RUN).unwrap();

    // (a) The evaluation alone: the walk at each character's phase, half of them fading
    // from the run.
    let mut spans = Vec::with_capacity(TICKS as usize);
    let mut bones = 0usize;
    for tick in 0..TICKS {
        spans.push(timed(|| {
            for n in 0..CHARACTERS {
                let at = |rate: u32| Phase((tick.wrapping_mul(rate)).wrapping_add(n as u32 * 977));
                let outgoing = (n % 2 == 0).then(|| Outgoing {
                    clip: run,
                    phase: at(95_443_717),
                    weight: Scalar::from_f64(0.4),
                });
                let pose = SimPose::evaluate(skeleton, walk, at(71_582_788), outgoing);
                bones += black_box(pose).len();
            }
        }));
    }
    assert_eq!(bones, (CHARACTERS * u64::from(BONES)) as usize * TICKS as usize);
    let best = spans.iter().min().copied().unwrap();
    println!(
        "(a) SimPose::evaluate, 200 characters, half blending: median {:.1} us, best {:.1} us per tick",
        micros(median(spans)),
        micros(best)
    );

    // (b) The step with and without the pose, on one and four workers.
    for workers in [1, 4] {
        let executor = executor(workers);
        let (with, world) = step_spans(&executor, step_world(true));
        assert_eq!(blending(world.world()), (CHARACTERS / 2) as usize);
        for n in 1..=CHARACTERS {
            assert_eq!(world.world().get::<SimPose>(key(n)).unwrap().len(), usize::from(BONES));
        }
        let (without, world) = step_spans(&executor, step_world(false));
        assert_eq!(blending(world.world()), (CHARACTERS / 2) as usize);
        let (with, without) = (median(with), median(without));
        println!(
            "(b) Simulation::step, {workers} worker(s): median {:.1} us with SimPose, {:.1} us \
             without, {:.1} us difference per tick",
            micros(with),
            micros(without),
            micros(with.saturating_sub(without))
        );
    }

    // (c) Bodies on every bone, against the same scene without them: over the physics test
    // backend with its contact search off (quadratic in the colliders, it would time the
    // backend), and with no physics at all, which leaves this crate's own share (the bone
    // commands, the report scan and their application at the sync point).
    for (workers, physics) in [(1, true), (4, true), (1, false), (4, false)] {
        let scene = |bones: bool| {
            let mut options = Options::new(WorldKind::Server);
            options.physics = physics;
            options.contacts = false;
            let mut fixture = options.build_on(executor(workers));
            fixture.assets(library(true).pin());
            for n in 1..=CHARACTERS {
                fixture.spawn(key(n), AnimRig::new(MACHINE, SKELETON));
                fixture.set_params(key(n), |p| *p = params(n));
            }
            if bones {
                let script = fixture.world_mut().service_mut::<Ragdoll>().unwrap();
                script.actions.extend((1..=CHARACTERS).map(|n| (2, key(n), RagdollAction::Spawn)));
            }
            fixture
        };
        let spans = |fixture: &mut Fixture| {
            fixture.run(WARM_UP);
            (0..TICKS).map(|_| timed(|| fixture.step())).collect::<Vec<_>>()
        };
        let mut with = scene(true);
        let with_spans = spans(&mut with);
        let rows = with.world().columns::<BoneLink>().map(|c| c.keys().len()).sum::<usize>();
        assert_eq!(rows, (CHARACTERS * u64::from(BONES)) as usize);
        let mut without = scene(false);
        let without_spans = spans(&mut without);
        let (with, without) = (median(with_spans), median(without_spans));
        let over = if physics { "test backend, contacts off" } else { "no physics" };
        println!(
            "(c) a body on every bone, {over}, {workers} worker(s): median {:.1} us with, \
             {:.1} us without, {:.1} us of bodies per tick",
            micros(with),
            micros(without),
            micros(with.saturating_sub(without))
        );
    }
}
