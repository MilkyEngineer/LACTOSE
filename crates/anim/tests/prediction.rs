//! The prediction world (`README.md`, "Prediction and replication"; "Proof for milestone
//! 1", the prediction row): the build check over this crate's systems beside a game sketch
//! driven by `PlayerInput`; a rollback that re-simulates an animated character from the
//! ring's inputs; a corrected `AnimState` that re-simulates from the server's phase; and
//! `AnimParams::quantise` over a sweep of speeds, the rarity of a straddled grid midpoint.

mod common;

use std::num::NonZeroU64;
use std::sync::Arc;

use common::{Library, MACHINE, SKELETON, chain, clip, clip_ref, state, transition, with_root};
use lactose_anim::{
    AnimParams, AnimRig, AnimState, Clip, Condition, RootMotion, SimPose, StateMachine, sets,
};
use lactose_asset_api::AssetRef;
use lactose_bundle::ComponentData;
use lactose_core::{NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{
    Executor, LayoutConfig, Mut, RegionConfig, RegionId, RegionMode, SystemSet, World, WorldKind,
    system,
};
use lactose_input::{Action, Actions, Axis2, PlayerInput, PlayerInputApplier};
use lactose_math::{Scalar, Vec3};
use lactose_net_api::{StagedFrame, StagedInbox, StagedSpawn};
use lactose_physics_api::Transform;
use lactose_predict::{
    CorrectionCause, DEFAULT_DEPTH, LocalPlayer, PredictError, Predictor, Reconciled,
    check_prediction_reads,
};
use lactose_reflect::{Persistence, Reflect, TypeRole, component};
use lactose_sim::harness::{ChunkCapacity, layout_mode};
use lactose_sim::{Registration, Simulation, SimulationBuilder, Stage, TickInput, TypeTable};

const PLAYER: PlayerId = PlayerId(1);
const REGION: RegionId = RegionId(1);
/// The prediction world is built at tick 10 and joins at the server's tick 4.
const START: Tick = Tick(10);
const JOIN: Tick = Tick(4);

const WALK_CLIP: AssetRef<Clip> = clip_ref("prediction/walk");
const JUMP_CLIP: AssetRef<Clip> = clip_ref("prediction/jump");
const JUMP: u8 = 0;
const SPEED: u8 = 0;

fn me() -> StableKey {
    StableKey::Net(NetId::runtime(NonZeroU64::new(1).expect("non-zero")).expect("runtime"))
}

// ---- The game sketch ----

/// The game's movement, after the pose: section 6.1's `apply_root_motion`.
static MOVEMENT: SystemSet = SystemSet::new("test::movement").after(&[&sets::EVALUATE]);

/// The speed parameter from the stick, a trigger from the jump's press edge: the inputs a
/// rollback replays.
#[system(name = "test::drive", worlds = [server, prediction], in_set = sets::DRIVE)]
fn drive(input: &PlayerInput, params: Mut<AnimParams>) {
    let mut next = *params;
    next.values[usize::from(SPEED)] = Scalar::from_i32(i32::from(input.movement.y)) / Scalar::TWO;
    if input.pressed.contains(Action::Jump) {
        next.trigger(JUMP);
    }
    params.set_if_neq(next);
}

#[system(name = "test::apply_root_motion", worlds = [server, prediction], in_set = MOVEMENT)]
fn apply_root_motion(motion: &RootMotion, root: Mut<Transform>) {
    let next = motion.apply(&root);
    root.set_if_neq(next);
}

/// Persistent and not replicated: what the build check refuses a prediction system.
#[component(key = "test::diary", persistent)]
#[derive(PartialEq, Eq, Debug, Default)]
struct Diary(#[field(id = 1)] u32);

#[system(name = "test::diarist", worlds = [server, prediction], in_set = sets::DRIVE)]
fn diarist(_params: &AnimParams, diary: Mut<Diary>) {
    diary.0 += 1;
}

fn register_game(registration: &mut Registration) {
    registration.stage(Stage::Simulation).sets(&[&MOVEMENT]).add(&DRIVE).add(&APPLY_ROOT_MOTION);
}

fn register_diarist(registration: &mut Registration) {
    registration.stage(Stage::Simulation).add(&DIARIST);
}

/// Walking at the speed of the stick, root motion forward; a one-shot jump on the trigger
/// from either state, back to walking when it finishes, over a blend.
fn library() -> Library {
    let mut walk = state("walk", WALK_CLIP, 1.0);
    walk.speed_param = Some(SPEED);
    let mut land = transition(Some(1), 0, &[Condition::Finished]);
    land.blend_ticks = 3;
    let machine = StateMachine {
        states: vec![walk, state("jump", JUMP_CLIP, 1.0)],
        transitions: vec![transition(None, 1, &[Condition::Trigger(JUMP)]), land],
        initial: 0,
    };
    Library::new()
        .with(SKELETON, chain(3, |_| false))
        .with(MACHINE, machine)
        .with(WALK_CLIP, with_root(clip(3, 6, 30, true, 0.0), 0.25, 0.05))
        .with(JUMP_CLIP, with_root(clip(3, 4, 8, false, 0.4), 0.1, 0.0))
}

fn types() -> TypeTable {
    let mut types = TypeTable::new();
    let register = |types: &mut TypeTable| -> Result<()> {
        types.register::<Diary>()?;
        lactose_input::register_types(types)?;
        lactose_net_api::register_types(types)?;
        lactose_predict::register_types(types)?;
        lactose_physics_api::register_types(types)?;
        lactose_anim::register_types(types)
    };
    register(&mut types).expect("the game registers");
    types
}

/// The prediction world `app` builds, with this crate's systems, the game sketch and
/// `extra`, under the build check.
fn build(layout: LayoutConfig, extra: fn(&mut Registration)) -> Result<Simulation> {
    let mut simulation = SimulationBuilder::new(WorldKind::Prediction, types())
        .layout(layout)
        .tick(START)
        .setup(lactose_net_api::setup)
        .systems(lactose_input::register_systems)
        .systems(lactose_net_api::register_systems)
        .systems(lactose_anim::register_systems)
        .systems(register_game)
        .systems(extra)
        .check(check_prediction_reads)
        .inputs(Box::new(PlayerInputApplier::new()))
        .build()?;
    let region = RegionConfig { id: REGION, seed: 11, mode: RegionMode::Simulated };
    simulation.world_mut().create_region(region)?;
    simulation.world_mut().set_region_assets(REGION, library().pin())?;
    Ok(simulation)
}

fn data<T: Reflect>(value: &T) -> ComponentData {
    ComponentData { component: T::KEY, bytes: bytes(value).to_vec() }
}

fn bytes<T: Reflect>(value: &T) -> &[u8] {
    lactose_reflect::bytemuck::bytes_of(value)
}

/// The sample the player sends for tick `t`: the stick forward by an amount that changes
/// every few ticks, the jump pressed every ninth tick.
fn sample(t: u32) -> PlayerInput {
    let y = i16::try_from(t % 5).expect("small") + 1;
    let jump = if t.is_multiple_of(9) { Actions::EMPTY.with(Action::Jump) } else { Actions::EMPTY };
    PlayerInput { movement: Axis2 { x: 0, y }, held: jump, pressed: jump, ..PlayerInput::IDLE }
}

/// The predictor across runs, as `app` keeps it.
struct Rig {
    predictor: Option<Predictor>,
    executor: Executor,
}

impl Rig {
    /// A rig joined at [`JOIN`]: the character claimed by the local player at the origin,
    /// entering the machine's initial state, applied by the first step and reconciled.
    fn joined(layout: LayoutConfig, workers: u32) -> Self {
        let simulation = build(layout, |_| {}).expect("the prediction world builds");
        let local = LocalPlayer { player: PLAYER, entity: me() };
        let predictor = Predictor::new(simulation, local, DEFAULT_DEPTH).expect("a predictor");
        let executor =
            Executor::builder().frame_workers(workers).background_workers(1).build().unwrap();
        let mut rig = Self { predictor: Some(predictor), executor };
        let mut frame = StagedFrame::resync(JOIN);
        let mut components = vec![
            data(&Transform::at(Vec3::ZERO)),
            data(&AnimRig::new(MACHINE, SKELETON)),
            data(&AnimState::UNSET),
            data(&AnimParams::ZERO),
            data(&RootMotion::ZERO),
            data(&SimPose::EMPTY),
        ];
        components.sort_by_key(|data| data.component);
        let id = NetId::runtime(NonZeroU64::new(1).expect("non-zero")).expect("runtime");
        frame
            .spawns
            .insert(id, StagedSpawn { origin: None, region: REGION, slot: None, components });
        rig.push(frame);
        rig.step();
        let reconciled = rig.reconcile();
        assert_eq!(reconciled.confirmed, Some(JOIN));
        assert!(reconciled.corrections.is_empty());
        assert!(rig.state().state().is_some(), "the machine and its clips resolve");
        assert_ne!(rig.world().get::<SimPose>(me()), Some(&SimPose::EMPTY), "the pose evaluates");
        rig
    }

    fn p(&self) -> &Predictor {
        self.predictor.as_ref().expect("kept")
    }

    fn world(&self) -> &World {
        self.p().simulation().world()
    }

    fn world_mut(&mut self) -> &mut World {
        self.predictor.as_mut().expect("kept").simulation_mut().world_mut()
    }

    fn tick(&self) -> u32 {
        self.p().tick().0
    }

    fn hash(&self) -> u64 {
        self.p().simulation().hash().expect("a hash")
    }

    fn state(&self) -> AnimState {
        *self.world().get::<AnimState>(me()).expect("the character's state")
    }

    fn root(&self) -> Transform {
        *self.world().get::<Transform>(me()).expect("the character's root")
    }

    fn push(&mut self, frame: StagedFrame) {
        let inbox = self.world_mut().service_mut::<StagedInbox>().expect("net-api's setup");
        assert!(inbox.push(Arc::new(frame)), "the inbox accepts the frame");
    }

    /// One predicted tick with [`sample`] of the tick it runs.
    fn step(&mut self) {
        let next = Tick(self.p().tick().0 + 1);
        let mut input = TickInput::default();
        input.inputs.insert(PLAYER, sample(next.0).to_player_sample(next));
        let predictor = self.predictor.take().expect("kept");
        match self.executor.block_on(predictor.step(&self.executor, input)) {
            Ok(predictor) => self.predictor = Some(predictor),
            Err(PredictError { error, .. }) => panic!("the step failed: {error}"),
        }
    }

    fn step_to(&mut self, tick: u32) {
        while self.tick() < tick {
            self.step();
        }
    }

    fn reconcile(&mut self) -> Reconciled {
        let predictor = self.predictor.take().expect("kept");
        match self.executor.block_on(predictor.reconcile(&self.executor)) {
            Ok((predictor, reconciled)) => {
                self.predictor = Some(predictor);
                reconciled
            }
            Err(PredictError { error, .. }) => panic!("the reconciliation failed: {error}"),
        }
    }
}

fn layouts() -> Vec<(LayoutConfig, u32)> {
    vec![
        (LayoutConfig::default(), 1),
        (layout_mode(ChunkCapacity::Rows(1), 3).expect("a layout"), 4),
        (layout_mode(ChunkCapacity::Rows(2), 7).expect("a layout"), 2),
    ]
}

/// The server's frame for tick `tick` stating the character as `world` holds it: every
/// persistent, replicated component, exact. A row fact for a claimed entity is a
/// whole-entity statement (`lactose-predict`'s reconciliation), so a component left out
/// would be removed by the correction.
fn statement(world: &World, tick: u32) -> StagedFrame {
    let mut frame = StagedFrame::new(Tick(tick));
    for info in world.registry().iter() {
        let recorded = info.role() == TypeRole::Component
            && info.persistence() == Persistence::Persistent
            && info.tracking().replicated;
        if let Some(bytes) = world.get_erased(me(), info.key()).filter(|_| recorded) {
            assert!(frame.push_row(me(), info.key(), frame.tick, true, bytes));
        }
    }
    frame
}

/// The tick the server's fact is for, and the tick the client has predicted to when the
/// frame arrives: a rollback of nine ticks, across a jump (tick 18) and its landing.
const FACT: u32 = 16;
const NOW: u32 = 25;

#[test]
fn the_build_check_passes_this_crates_systems_and_refuses_an_unreplicated_write() {
    build(LayoutConfig::default(), |_| {}).expect("the anim systems and the sketch pass");
    let Err(refused) = build(LayoutConfig::default(), register_diarist) else {
        panic!("a prediction system writing a persistent, unreplicated row passed");
    };
    let message = refused.to_string();
    assert!(message.contains("test::diarist") && message.contains("test::diary"), "{message}");
}

#[test]
fn a_rollback_resimulates_the_character_from_the_rings_inputs() {
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined(layout, workers);
        let mut straight = Rig::joined(layout, workers);
        rig.step_to(NOW - 1);
        // The server's fact: the root a metre to the side at tick 16, which the straight
        // run takes as it happens and the rig learns of nine ticks late.
        straight.step_to(FACT);
        let moved =
            Transform { translation: straight.root().translation + Vec3::X, ..straight.root() };
        straight.world_mut().insert(me(), moved).unwrap();
        let frame = statement(straight.world(), FACT);
        let mut states = Vec::new();
        while straight.tick() < NOW {
            straight.step();
            states.push(straight.state().state());
        }
        assert!(states.contains(&Some(0)) && states.contains(&Some(1)), "{states:?}");

        rig.push(frame);
        rig.step();
        let before = rig.state();
        let reconciled = rig.reconcile();
        let corrections: Vec<_> = reconciled
            .corrections
            .iter()
            .map(|c| (c.tick, c.depth, c.cause, c.entities.clone(), c.divergence.fields))
            .collect();
        let one_field = (Tick(FACT), NOW - FACT, CorrectionCause::Mismatch, vec![me()], 1);
        assert_eq!(corrections, [one_field], "the root alone differed");
        assert_eq!(reconciled.resimulated, NOW - FACT);
        assert_eq!(rig.tick(), NOW);
        assert_eq!(rig.state(), before, "the animation re-simulated to what it predicted");
        assert_eq!(rig.state(), straight.state());
        assert_eq!(rig.root(), straight.root());
        assert_eq!(rig.hash(), straight.hash());
        // And they stay equal.
        rig.step_to(NOW + 20);
        straight.step_to(NOW + 20);
        assert_eq!(rig.hash(), straight.hash());
    }
}

#[test]
fn a_corrected_state_resimulates_from_the_servers_phase() {
    for (layout, workers) in layouts() {
        let mut rig = Rig::joined(layout, workers);
        let mut reference = Rig::joined(layout, workers);
        // The server's state at tick 14 is the one the character had at tick 12: the
        // client mispredicted the phase by two ticks of walking, and learns it at 17,
        // before the jump at 18 enters at zero whatever the phase was.
        let (was, fact, now) = (12, 14, 17);
        reference.step_to(was);
        let server = reference.state();
        reference.step_to(fact);
        assert_ne!(reference.state().phase(), server.phase());
        reference.world_mut().insert(me(), server).unwrap();
        let frame = statement(reference.world(), fact);
        reference.step_to(now);

        rig.step_to(now - 1);
        rig.push(frame);
        rig.step();
        let predicted = (rig.state(), rig.root());
        let reconciled = rig.reconcile();
        let corrections: Vec<_> = reconciled
            .corrections
            .iter()
            .map(|c| (c.tick, c.depth, c.cause, c.entities.clone()))
            .collect();
        assert_eq!(corrections, [(Tick(fact), now - fact, CorrectionCause::Mismatch, vec![me()])]);
        assert_ne!(rig.state().phase(), predicted.0.phase(), "the correction moved the phase");
        assert_eq!(rig.state(), reference.state());
        // The root motion followed the server's phase too.
        assert_ne!(rig.root(), predicted.1);
        assert_eq!(rig.root(), reference.root());
        assert_eq!(rig.hash(), reference.hash());
        // And they stay equal, across the jumps that follow.
        while rig.tick() < now + 30 {
            rig.step();
            reference.step();
            assert_eq!(rig.hash(), reference.hash());
        }
    }
}

/// The sweep's denominator: speeds are `8 i / P` for `i` in `-P..P`. `P` is odd, so
/// `16 * 8 i / P` is never a half-integer and no speed sits on a grid midpoint; the
/// nearest is `1 / (32 P)` away, far above the residuals' rounding.
const P: i64 = 100_003;
const STEPS: u32 = 16;

/// Whether an odd multiple of `P` lies strictly between `256 a` and `256 b`: whether the
/// interval of speeds `8 a / P` to `8 b / P` contains a grid midpoint `(2k + 1) / 32`,
/// in integers.
fn straddles(a: i64, b: i64) -> bool {
    let (low, high) = (256 * a.min(b), 256 * a.max(b));
    // The odd multiples at most `x` number `floor((floor(x / P) + 1) / 2)` up to a constant.
    let odd = |x: i64| (x.div_euclid(P) + 1).div_euclid(2);
    odd(high) != odd(low)
}

#[test]
fn a_quantised_speed_differs_only_where_a_residual_straddles_a_grid_midpoint() {
    let p = Scalar::from_i64(P);
    let sweep = 2 * P;
    // Residuals as whole steps of the sweep (8 / P, about 8e-5): from one step to about
    // 2e-3, the size of `PredictionTolerance::DEFAULT`, either way.
    for steps in [1i64, -1, 4, -7, 12, 25, -25] {
        let residual = Scalar::from_i64(8 * steps) / p;
        let mut differing = 0;
        for i in -P..P {
            let speed = Scalar::from_i64(8 * i) / p;
            let solver = AnimParams::quantise(speed, STEPS);
            let other = AnimParams::quantise(speed + residual, STEPS);
            let changed = solver.to_canonical_bits() != other.to_canonical_bits();
            assert_eq!(changed, straddles(i, i + steps), "speed {i} / {P} * 8, {steps} steps");
            differing += i64::from(changed);
        }
        // About `16 |r|` of the sweep: 3.2 % at 2e-3.
        let midpoints: i64 = (-P..P).map(|i| i64::from(straddles(i, i + steps))).sum();
        assert_eq!(differing, midpoints);
        assert!(differing > 0, "{steps} steps straddle somewhere");
        assert!(differing * 25 < sweep, "{differing} of {sweep} at {steps} steps: rare");
    }
    // A residual of a unit in the last place straddles nothing in the sweep.
    for i in -P..P {
        let speed = Scalar::from_i64(8 * i) / p;
        let nudged = speed + speed.abs() * Scalar::EPSILON;
        assert_eq!(
            AnimParams::quantise(speed, STEPS).to_canonical_bits(),
            AnimParams::quantise(nudged, STEPS).to_canonical_bits()
        );
    }
}
