//! The sim skeleton's evaluation (`README.md`, "The sim skeleton"; "Proof for milestone 1",
//! the evaluation row): `evaluate` against a hand-composed two-bone pose at authored phases,
//! with and without a blend, bit for bit; `rest`; a clip for another skeleton; a limp
//! character's row in a server and a prediction world; the same bits under every layout and
//! worker count.

mod common;

use common::{
    Fixture, Library, MACHINE, Options, RagdollAction, SKELETON, chain, clip, clip_ref, key, s,
    sample, state, transition,
};
use lactose_anim::{
    AnimRig, BonePose, Bones, Clip, Condition, Outgoing, Phase, SimPose, Skeleton, StateMachine,
};
use lactose_asset_api::{AssetId, AssetRef};
use lactose_ecs::{LayoutConfig, WorldKind};
use lactose_math::{Scalar, ScalarBits};
use lactose_physics_api::Transform;
use lactose_sim::harness::{ChunkCapacity, layout_mode};

/// A pose's scalars by their canonical bits, so equality is bit for bit.
fn bits(pose: &BonePose) -> [ScalarBits; 7] {
    let (t, r) = (pose.translation, pose.rotation);
    [t.x, t.y, t.z, r.x, r.y, r.z, r.w].map(Scalar::to_canonical_bits)
}

fn pose_bits(pose: &SimPose) -> Vec<[ScalarBits; 7]> {
    pose.bones().iter().map(bits).collect()
}

/// The pose `Clip::sample` reads at `phase` of a loop of `frames` frames whose samples are
/// `common::sample` with `salt`, computed by hand from the contract: the frame below the
/// phase blended toward the next by the fraction, one division.
fn hand_sample(frames: u32, salt: f64, phase: Phase, bone: u8) -> BonePose {
    let position = u64::from(phase.0) * u64::from(frames);
    let frame = (position >> 32) as u32;
    let fraction = position & ((1 << 32) - 1);
    let a = sample(frame, bone, salt);
    if fraction == 0 {
        return a;
    }
    let b = sample((frame + 1) % frames, bone, salt);
    a.blend(b, Scalar::from_u64(fraction) / Scalar::from_u64(1 << 32))
}

#[test]
fn evaluate_equals_a_hand_composed_two_bone_pose() {
    let skeleton = chain(2, |_| false);
    let current = clip(2, 4, 40, true, 0.0);
    let other = clip(2, 4, 40, true, 0.4);
    let phases = [
        Phase::ZERO,
        Phase(1 << 30),
        Phase::HALF,
        Phase(3 << 30),
        Phase((1 << 30) + (1 << 27)),
        Phase(123_456_789),
        Phase::END,
    ];
    for phase in phases {
        // Without a blend: bone zero is its sample, bone one its sample composed onto it.
        let root = hand_sample(4, 0.0, phase, 0);
        let hand = [root, root.compose(hand_sample(4, 0.0, phase, 1))];
        let pose = SimPose::evaluate(&skeleton, &current, phase, None);
        assert_eq!(pose_bits(&pose), hand.iter().map(bits).collect::<Vec<_>>(), "{phase:?}");
        assert_eq!(pose.len(), 2);
        for bone in 0..2 {
            assert_eq!(bits(&current.sample(phase, bone)), bits(&hand_sample(4, 0.0, phase, bone)));
        }

        // With a blend toward the other clip at another phase.
        let out_phase = Phase(phase.0.wrapping_add(987_654_321));
        let weight = s(0.375);
        let local = |bone| {
            hand_sample(4, 0.0, phase, bone).blend(hand_sample(4, 0.4, out_phase, bone), weight)
        };
        let root = local(0);
        let hand = [root, root.compose(local(1))];
        let outgoing = Outgoing { clip: &other, phase: out_phase, weight };
        let pose = SimPose::evaluate(&skeleton, &current, phase, Some(outgoing));
        assert_eq!(
            pose_bits(&pose),
            hand.iter().map(bits).collect::<Vec<_>>(),
            "{phase:?} blended"
        );
    }
}

#[test]
fn a_one_shot_at_its_end_samples_its_last_frame() {
    let one_shot = clip(2, 5, 30, false, 0.1);
    for bone in 0..2 {
        assert_eq!(bits(&one_shot.sample(Phase::END, bone)), bits(&sample(4, bone, 0.1)));
        assert_eq!(bits(&one_shot.sample(Phase::ZERO, bone)), bits(&sample(0, bone, 0.1)));
    }
}

#[test]
fn rest_composes_the_rest_poses() {
    let skeleton = chain(3, |_| false);
    let rest = SimPose::rest(&skeleton);
    let r = |bone: usize| skeleton.bones[bone].rest;
    let hand = [r(0), r(0).compose(r(1)), r(0).compose(r(1)).compose(r(2))];
    assert_eq!(pose_bits(&rest), hand.iter().map(bits).collect::<Vec<_>>());
    assert_eq!(SimPose::rest(&Skeleton { bones: Vec::new(), sockets: Vec::new() }), SimPose::EMPTY);
}

#[test]
fn a_clip_of_another_bone_count_gives_empty() {
    let skeleton = chain(3, |_| false);
    let two = clip(2, 4, 40, true, 0.0);
    assert_eq!(SimPose::evaluate(&skeleton, &two, Phase::HALF, None), SimPose::EMPTY);
    let three = clip(3, 4, 40, true, 0.0);
    let outgoing = Outgoing { clip: &two, phase: Phase::ZERO, weight: s(0.5) };
    assert_eq!(SimPose::evaluate(&skeleton, &three, Phase::HALF, Some(outgoing)), SimPose::EMPTY);
    assert_eq!(SimPose::evaluate(&skeleton, &three, Phase::HALF, None).len(), 3);
}

const LOOP_CLIP: AssetRef<Clip> = clip_ref("evaluation/loop");
const OTHER_CLIP: AssetRef<Clip> = clip_ref("evaluation/other");
const OTHER_SKELETON: AssetRef<Skeleton> = AssetRef::new(AssetId::from_name("evaluation/other"));

/// A loop and a second state it cross-fades into on trigger zero over eight ticks.
fn library(bones: usize, bodies: fn(usize) -> bool) -> Library {
    let mut cross = transition(Some(0), 1, &[Condition::Trigger(0)]);
    cross.blend_ticks = 8;
    let mut back = transition(Some(1), 0, &[Condition::Trigger(0)]);
    back.blend_ticks = 8;
    back.entry_phase = Phase::HALF;
    let machine = StateMachine {
        states: vec![state("loop", LOOP_CLIP, 1.0), state("other", OTHER_CLIP, 1.3)],
        transitions: vec![cross, back],
        initial: 0,
    };
    let count = bones as u8;
    Library::new()
        .with(SKELETON, chain(bones, bodies))
        .with(MACHINE, machine)
        .with(LOOP_CLIP, clip(count, 6, 50, true, 0.0))
        .with(OTHER_CLIP, clip(count, 3, 20, true, 0.7))
}

#[test]
fn the_system_writes_the_rows_evaluation_and_empty_for_another_skeletons_clip() {
    let library = library(3, |_| false);
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library.pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    let assets = library.pin();
    for tick in 0..30 {
        if tick == 10 {
            fixture.set_params(key(1), |params| params.trigger(0));
        }
        fixture.step();
        let row = fixture.state(key(1));
        let clip = |index| if index == 0 { assets.get(LOOP_CLIP) } else { assets.get(OTHER_CLIP) };
        let skeleton = assets.get(SKELETON).unwrap();
        let outgoing = row.blend().map(|blend| Outgoing {
            clip: clip(blend.from).unwrap(),
            phase: blend.phase,
            weight: blend.weight,
        });
        let expected =
            SimPose::evaluate(skeleton, clip(row.state().unwrap()).unwrap(), row.phase(), outgoing);
        assert_eq!(pose_bits(&fixture.pose(key(1))), pose_bits(&expected), "tick {tick}");
        assert_eq!(fixture.seen(key(1)).simulation_bones, 3);
    }

    // The rig names another skeleton than the one the clips were cooked for.
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library.clone().with(OTHER_SKELETON, chain(3, |_| false)).pin());
    fixture.spawn(key(1), AnimRig::new(MACHINE, OTHER_SKELETON));
    fixture.run(3);
    assert!(fixture.state(key(1)).state().is_some(), "the machine runs");
    assert_eq!(fixture.pose(key(1)), SimPose::EMPTY);

    // No machine: the state stays unset and the pose empty.
    let mut fixture = Fixture::new(WorldKind::Server);
    fixture.assets(library.pin_without(&[MACHINE.id()]));
    fixture.spawn(key(1), AnimRig::new(MACHINE, SKELETON));
    fixture.run(3);
    assert_eq!(fixture.state(key(1)).state(), None);
    assert_eq!(fixture.pose(key(1)), SimPose::EMPTY);
}

/// Bodies on every bone but the third, so the fit composes one bone onto its parent.
fn bodies(bone: usize) -> bool {
    bone != 2
}

fn ragdoll_fixture(kind: WorldKind) -> Fixture {
    let mut options = Options::new(kind);
    options.physics = true;
    let mut fixture = options.build();
    fixture.assets(library(4, bodies).pin());
    fixture.spawn_at(
        key(1),
        AnimRig::new(MACHINE, SKELETON),
        Transform::at(common::v(2.0, 0.0, -1.0)),
    );
    fixture
}

#[test]
fn a_limp_characters_pose_is_empty_after_evaluate_and_fitted_after_fit_in_a_server_world() {
    let mut fixture = ragdoll_fixture(WorldKind::Server);
    let actions = vec![(1, key(1), RagdollAction::Spawn), (3, key(1), RagdollAction::Limp(true))];
    fixture.world_mut().service_mut::<common::Ragdoll>().unwrap().actions = actions;
    fixture.run(3);
    assert_eq!(
        fixture.seen(key(1)),
        common::Seen { events: 0, simulation_bones: 4, post_bones: 4 }
    );
    let assets = library(4, bodies).pin();
    let skeleton = assets.get(SKELETON).unwrap();
    let clip = assets.get(LOOP_CLIP).unwrap();
    for tick in 4..12 {
        fixture.step();
        let seen = fixture.seen(key(1));
        assert_eq!((seen.simulation_bones, seen.post_bones), (0, 4), "tick {tick}");
        let bones = *fixture.world().get::<Bones>(key(1)).unwrap();
        assert!(bones.is_limp());
        let root = *fixture.world().get::<Transform>(key(1)).unwrap();
        let to_root = root.rotation.conjugate();
        let phase = fixture.state(key(1)).phase();
        let mut hand: Vec<BonePose> = Vec::new();
        for bone in 0..4u8 {
            let fitted = match bones.key(bone) {
                Some(row) => {
                    let world = *fixture.world().get::<Transform>(row).unwrap();
                    BonePose::new(
                        to_root * (world.translation - root.translation),
                        to_root * world.rotation,
                    )
                }
                None => hand[usize::from(bone) - 1].compose(clip.sample(phase, bone)),
            };
            hand.push(fitted);
        }
        assert_eq!(pose_bits(&fixture.pose(key(1))), hand.iter().map(bits).collect::<Vec<_>>());
        // The bodies fall, so the fit is not the animation.
        assert_ne!(fixture.pose(key(1)), SimPose::evaluate(skeleton, clip, phase, None));
    }
}

#[test]
fn a_limp_characters_pose_is_empty_all_tick_in_a_prediction_world() {
    // The server's limp `Bones`, as replication would deliver it.
    let mut server = ragdoll_fixture(WorldKind::Server);
    let actions = vec![(1, key(1), RagdollAction::Spawn), (2, key(1), RagdollAction::Limp(true))];
    server.world_mut().service_mut::<common::Ragdoll>().unwrap().actions = actions;
    server.run(3);
    let bones = *server.world().get::<Bones>(key(1)).unwrap();
    assert!(bones.is_limp());

    let mut fixture = ragdoll_fixture(WorldKind::Prediction);
    fixture.run(2);
    assert_eq!(fixture.seen(key(1)).simulation_bones, 4, "the prediction world evaluates");
    fixture.world_mut().insert(key(1), bones).unwrap();
    for _ in 0..4 {
        fixture.step();
        let seen = fixture.seen(key(1));
        assert_eq!((seen.simulation_bones, seen.post_bones), (0, 0));
        assert_eq!(fixture.pose(key(1)), SimPose::EMPTY);
        assert!(fixture.log().moved.is_empty());
    }
}

/// Every character's pose bits and the world's hash, tick by tick.
fn poses(layout: LayoutConfig, workers: u32) -> Vec<(Vec<Vec<[ScalarBits; 7]>>, u64)> {
    const CHARACTERS: u64 = 16;
    let mut options = Options::new(WorldKind::Server);
    options.layout = layout;
    options.workers = workers;
    let mut fixture = options.build();
    fixture.assets(library(5, |_| false).pin());
    for n in 0..CHARACTERS {
        fixture.spawn(key(n + 1), AnimRig::new(MACHINE, SKELETON));
    }
    let mut out = Vec::new();
    for tick in 0..120u64 {
        for n in 0..CHARACTERS {
            if (tick + 3 * n) % 29 == 0 {
                fixture.set_params(key(n + 1), |params| params.trigger(0));
            }
        }
        fixture.step();
        let bits = (0..CHARACTERS).map(|n| pose_bits(&fixture.pose(key(n + 1)))).collect();
        out.push((bits, fixture.hash()));
    }
    out
}

#[test]
fn the_same_bits_under_every_layout_and_worker_count() {
    let reference = poses(LayoutConfig::default(), 1);
    for layout in [
        LayoutConfig::default(),
        layout_mode(ChunkCapacity::Rows(2), 3).unwrap(),
        layout_mode(ChunkCapacity::Bytes(16384), 5).unwrap(),
        layout_mode(ChunkCapacity::Production, 9).unwrap(),
    ] {
        for workers in [1, 4] {
            assert!(poses(layout, workers) == reference, "{layout:?}, {workers} workers");
        }
    }
}
