//! Bones and ragdolls (`README.md`, "Bones and ragdolls"; "Proof for milestone 1", the
//! bones row): the rows `Bones::spawn` records and nothing on a second call; the three
//! `Stage::Post` systems in a server world and not in a prediction world; the kinematic
//! bodies swept to the previous tick's pose as the physics test backend sees it; `set_limp`
//! both ways; `report_bones` from limp rows alone; the collider rule and the body rules
//! `Skeleton::validate` checks; every body accepted when the chain's composed rotations
//! leave the unit tolerance; and no command for a character without bones. The fit itself
//! is in `tests/evaluation.rs`.

#![forbid(unsafe_code)]

mod common;

use common::fake::Call;
use common::{
    Fixture, Library, MACHINE, Options, Ragdoll, RagdollAction, SKELETON, bone_body, chain, clip,
    clip_ref, key, s, state, v,
};
use lactose_anim::{AnimRig, BoneBody, BoneLink, Bones, Clip, SimPose, Skeleton, StateMachine};
use lactose_asset_api::AssetRef;
use lactose_core::{ErrorKind, StableKey};
use lactose_ecs::{ClaimedBy, OwnedBy, WorldKind};

use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{
    BodyType, Collider, CollisionGroups, Joint, JointKind, RigidBody, Shape, Transform, locked_axes,
};

const LOOP_CLIP: AssetRef<Clip> = clip_ref("bones/loop");
const BONES: usize = 4;
const HERO: u64 = 1;

/// Bodies on every bone but the third: the fourth is jointed to the second.
fn bodies(bone: usize) -> bool {
    bone != 2
}

fn library() -> Library {
    let machine = StateMachine {
        states: vec![state("loop", LOOP_CLIP, 1.0)],
        transitions: Vec::new(),
        initial: 0,
    };
    Library::new()
        .with(SKELETON, chain(BONES, bodies))
        .with(MACHINE, machine)
        .with(LOOP_CLIP, clip(BONES as u8, 6, 30, true, 0.0))
}

fn root() -> Transform {
    Transform::new(v(1.0, 0.5, -3.0), common::yaw(0.4))
}

/// A world with physics behind the test backend, the hero and two characters without
/// bones, and `actions` for the ragdoll kernel.
fn fixture(kind: WorldKind, actions: &[(u32, u64, RagdollAction)]) -> Fixture {
    let mut options = Options::new(kind);
    options.physics = true;
    let mut fixture = options.build();
    fixture.assets(library().pin());
    fixture.spawn_at(key(HERO), AnimRig::new(MACHINE, SKELETON), root());
    fixture.spawn(key(2), AnimRig::new(MACHINE, SKELETON));
    fixture.spawn(key(3), AnimRig::new(MACHINE, SKELETON));
    fixture.world_mut().service_mut::<Ragdoll>().unwrap().actions =
        actions.iter().map(|&(tick, n, action)| (tick, key(n), action)).collect();
    fixture
}

fn bones_of(fixture: &Fixture, n: u64) -> Bones {
    *fixture.world().get::<Bones>(key(n)).unwrap()
}

fn rows<T: lactose_reflect::Component>(fixture: &Fixture) -> Vec<StableKey> {
    let mut keys: Vec<StableKey> =
        fixture.world().columns::<T>().flat_map(|column| column.keys().to_vec()).collect();
    keys.sort_unstable();
    keys
}

/// `transform` with its rotation normalised, as a bone row's is written.
fn normalised(transform: Transform) -> Transform {
    Transform::new(transform.translation, transform.rotation.normalize())
}

#[test]
fn spawn_records_the_stated_rows_and_nothing_on_a_second_call() {
    let actions = [(1, HERO, RagdollAction::Spawn), (4, HERO, RagdollAction::Spawn)];
    let mut fixture = fixture(WorldKind::Server, &actions);
    fixture.step();
    let pose = fixture.pose(key(HERO));
    assert_eq!(pose.len(), BONES);
    let skeleton = chain(BONES, bodies);
    let bones = bones_of(&fixture, HERO);
    assert_eq!((bones.len(), bones.is_limp()), (BONES, false));
    assert_eq!(bones.key(2), None, "a bone without a body has no row");
    assert_eq!(bones.key(BONES as u8), None);
    let row = |bone: u8| bones.key(bone).unwrap();
    for bone in [0u8, 1, 3] {
        let key = row(bone);
        let StableKey::Net(id) = key else { panic!("{key:?} is not a Net key") };
        assert!(!id.is_cooked(), "a runtime NetId, resolved from its placeholder");
        let world = fixture.world();
        let expected = normalised(pose.bones()[usize::from(bone)].in_world(&root()));
        assert_eq!(*world.get::<Transform>(key).unwrap(), expected, "bone {bone} at the pose");
        let body = skeleton.bones[usize::from(bone)].body.unwrap();
        let mut kinematic = body.body;
        kinematic.body_type = BodyType::Kinematic;
        assert_eq!(*world.get::<RigidBody>(key).unwrap(), kinematic);
        assert_eq!(*world.get::<Collider>(key).unwrap(), body.collider);
        assert_eq!(world.get::<OwnedBy>(key).unwrap().owner.get(), Some(common::key(HERO)));
        assert_eq!(*world.get::<BoneLink>(key).unwrap(), BoneLink::new(common::key(HERO), bone));
        assert!(world.get::<ClaimedBy>(key).is_none(), "no claim passes on");
    }
    // Joints: the second bone to the first, the fourth to the second (the third has no
    // body), each owned by its bone's row.
    let joints = rows::<Joint>(&fixture);
    assert_eq!(joints.len(), 2);
    let mut pairs: Vec<_> = joints
        .iter()
        .map(|&joint| {
            let world = fixture.world();
            let row = world.get::<Joint>(joint).unwrap();
            let owner = world.get::<OwnedBy>(joint).unwrap().owner.get().unwrap();
            assert_eq!(owner, row.body_b.get().unwrap(), "owned by the bone's row");
            (row.body_a.get().unwrap(), row.body_b.get().unwrap())
        })
        .collect();
    pairs.sort_unstable();
    let mut expected = vec![(row(0), row(1)), (row(1), row(3))];
    expected.sort_unstable();
    assert_eq!(pairs, expected);
    assert_eq!(rows::<BoneLink>(&fixture).len(), 3);

    // The second call, at tick four, spawns nothing and leaves `Bones` as it was.
    fixture.run(4);
    assert_eq!(rows::<BoneLink>(&fixture).len(), 3);
    assert_eq!(rows::<Joint>(&fixture).len(), 2);
    assert_eq!(bones_of(&fixture, HERO), bones);
}

/// The `SetTransform` calls of the step's sync, by key.
fn sweeps(calls: &[Call]) -> Vec<(StableKey, Transform)> {
    let mut sweeps: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::SetTransform(key, transform) => Some((*key, *transform)),
            _ => None,
        })
        .collect();
    sweeps.sort_unstable_by_key(|&(key, _)| key);
    sweeps
}

#[test]
fn drive_bones_sweeps_the_kinematic_bodies_to_the_previous_ticks_pose() {
    let mut fixture = fixture(WorldKind::Server, &[(1, HERO, RagdollAction::Spawn)]);
    fixture.run(2);
    let bones = bones_of(&fixture, HERO);
    let body_rows: Vec<StableKey> = [0u8, 1, 3].map(|bone| bones.key(bone).unwrap()).to_vec();
    for tick in 3..12 {
        let pose = fixture.pose(key(HERO));
        let root = *fixture.world().get::<Transform>(key(HERO)).unwrap();
        fixture.calls.take();
        fixture.step();
        let calls = fixture.calls.take();
        let mut expected: Vec<_> = [0u8, 1, 3]
            .iter()
            .map(|&bone| (bones.key(bone).unwrap(), normalised(pose.world(&root, bone).unwrap())))
            .collect();
        expected.sort_unstable_by_key(|&(key, _)| key);
        // Only the hero's bone rows are swept: the two characters without bones cost no
        // command and so no sweep.
        assert_eq!(sweeps(&calls), expected, "tick {tick}: last tick's pose");
        assert!(calls.iter().filter_map(Call::key).all(|key| body_rows.contains(&key)));
        // The pose moved, so the sweep does every tick.
        assert_ne!(fixture.pose(key(HERO)), pose);
    }
}

#[test]
fn set_limp_makes_the_bodies_dynamic_and_back() {
    let actions = [
        (1, HERO, RagdollAction::Spawn),
        (3, HERO, RagdollAction::Limp(true)),
        (3, 2, RagdollAction::Spawn),
        (8, HERO, RagdollAction::Limp(false)),
    ];
    let mut fixture = fixture(WorldKind::Server, &actions);
    fixture.run(3);
    let bones = bones_of(&fixture, HERO);
    assert!(bones.is_limp());
    let skeleton = chain(BONES, bodies);
    for bone in [0u8, 1, 3] {
        let row = bones.key(bone).unwrap();
        let mut dynamic = skeleton.bones[usize::from(bone)].body.unwrap().body;
        dynamic.body_type = BodyType::Dynamic;
        assert_eq!(*fixture.world().get::<RigidBody>(row).unwrap(), dynamic);
        assert!(fixture.world().get::<BoneLink>(row).unwrap().limp);
    }
    // The physics sync pushes the change of each body at the next tick.
    fixture.calls.take();
    fixture.step();
    let changed: Vec<StableKey> = fixture
        .calls
        .take()
        .iter()
        .filter_map(|call| match call {
            Call::SetBody(key, Some(body)) if body.body_type == BodyType::Dynamic => Some(*key),
            _ => None,
        })
        .collect();
    assert_eq!(changed.len(), 3);
    // Limp: the bodies fall, no sweep names them.
    for _ in 5..8 {
        fixture.step();
        assert!(sweeps(&fixture.calls.take()).iter().all(|(key, _)| !changed.contains(key)));
    }
    // Back: kinematic, not limp, swept to the pose again from wherever they lie.
    fixture.step();
    let bones = bones_of(&fixture, HERO);
    assert!(!bones.is_limp());
    for bone in [0u8, 1, 3] {
        let row = bones.key(bone).unwrap();
        assert_eq!(fixture.world().get::<RigidBody>(row).unwrap().body_type, BodyType::Kinematic);
        assert!(!fixture.world().get::<BoneLink>(row).unwrap().limp);
    }
    fixture.calls.take();
    fixture.step();
    // The tick that made them kinematic was still limp in its own `Stage::Post`, so its sync
    // sweeps the second character's rows alone; the hero's follow a tick later.
    assert_eq!(sweeps(&fixture.calls.take()).len(), 3, "the second character's rows");
    fixture.step();
    assert_eq!(sweeps(&fixture.calls.take()).len(), 3 + 3, "the hero's rows and the second's");
}

#[test]
fn report_bones_emits_from_limp_rows_alone() {
    let actions = [
        (1, HERO, RagdollAction::Spawn),
        (1, 2, RagdollAction::Spawn),
        (3, HERO, RagdollAction::Limp(true)),
    ];
    let mut fixture = fixture(WorldKind::Server, &actions);
    fixture.run(3);
    assert!(fixture.log().moved.is_empty(), "animated bones report nothing");
    let bones = bones_of(&fixture, HERO);
    for _ in 0..4 {
        let tick = fixture.tick() + 1;
        fixture.step();
        let mut moved: Vec<_> = fixture
            .log()
            .moved
            .iter()
            .filter(|(at, _)| *at == tick)
            .map(|(_, event)| *event)
            .collect();
        moved.sort_unstable_by_key(|event| event.bone);
        assert_eq!(moved.len(), 3, "one per limp bone row, none from the second's");
        for event in moved {
            assert_eq!(event.owner, key(HERO));
            let row = bones.key(event.bone).unwrap();
            assert_eq!(event.transform, *fixture.world().get::<Transform>(row).unwrap());
        }
    }
}

#[test]
fn the_post_systems_run_in_a_server_world_and_not_in_a_prediction_world() {
    // The server's rows, copied into a prediction world as if they had arrived: first the
    // character limp, then animated again.
    let actions = [
        (1, HERO, RagdollAction::Spawn),
        (2, HERO, RagdollAction::Limp(true)),
        (4, HERO, RagdollAction::Limp(false)),
    ];
    let mut server = fixture(WorldKind::Server, &actions);
    server.run(3);
    assert!(!server.log().moved.is_empty(), "the server reports");
    let limp = bones_of(&server, HERO);
    assert!(limp.is_limp());
    let body_rows: Vec<StableKey> = [0u8, 1, 3].map(|bone| limp.key(bone).unwrap()).to_vec();
    let arrived: Vec<(Transform, BoneLink)> = body_rows
        .iter()
        .map(|&row| {
            let world = server.world();
            (*world.get::<Transform>(row).unwrap(), *world.get::<BoneLink>(row).unwrap())
        })
        .collect();
    server.run(2);
    let animated = bones_of(&server, HERO);
    assert!(!animated.is_limp());

    let mut predicted = fixture(WorldKind::Prediction, &actions);
    predicted.run(1);
    predicted.world_mut().insert(key(HERO), limp).unwrap();
    for (&row, &(transform, link)) in body_rows.iter().zip(&arrived) {
        assert!(link.limp);
        predicted.world_mut().spawn(row, common::REGION, (transform, link)).unwrap();
    }
    let placed = |fixture: &Fixture| -> Vec<Transform> {
        body_rows.iter().map(|&row| *fixture.world().get::<Transform>(row).unwrap()).collect()
    };
    let before = placed(&predicted);
    predicted.run(3);
    assert!(predicted.log().moved.is_empty(), "no report in the prediction world");
    assert_eq!(predicted.pose(key(HERO)), SimPose::EMPTY, "no fit");
    assert_eq!(predicted.seen(key(HERO)).post_bones, 0);

    // Animated: the pose is evaluated, and no sweep writes the rows.
    predicted.world_mut().insert(key(HERO), animated).unwrap();
    predicted.run(3);
    assert_eq!(predicted.pose(key(HERO)).len(), BONES);
    assert_eq!(placed(&predicted), before, "no sweep in the prediction world");
    assert!(predicted.calls.all().iter().all(|call| !matches!(call, Call::SetTransform(..))));
}

#[test]
fn validate_refuses_bodies_whose_memberships_meet_their_filters() {
    let with_groups = |groups: [(u32, u32); 2]| {
        let mut skeleton = chain(2, |_| true);
        for (bone, (membership, filter)) in skeleton.bones.iter_mut().zip(groups) {
            let body = bone.body.as_mut().unwrap();
            body.collider = body.collider.with_groups(CollisionGroups::new(membership, filter));
        }
        skeleton
    };
    let refused = |skeleton: &Skeleton| {
        let error = skeleton.validate().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    };
    assert!(chain(BONES, bodies).validate().is_ok());
    assert!(with_groups([(1 << 1, 1 << 2), (1 << 1, 1 << 3)]).validate().is_ok());
    // The pair: each one's filter meets the other's membership.
    refused(&with_groups([(1, 2), (2, 1)]));
    // One bone touching its own group set.
    refused(&with_groups([(1 << 1, 1 << 1), (1 << 3, 1 << 4)]));
    // A filter that admits another bone's membership.
    refused(&with_groups([(1 << 1, 1 << 2), (1 << 2, 1 << 5)]));
    // A joint with no ancestor body is refused too; a body without a joint is not.
    let mut orphan = chain(2, |_| true);
    orphan.bones[0].body = Some(bone_body(true));
    refused(&orphan);
    let mut free = chain(2, |_| true);
    free.bones[1].body = Some(bone_body(false));
    assert!(free.validate().is_ok());
}

/// `Skeleton::validate` refuses a body the physics backend would refuse at every sync, one
/// case per class of `physics-rapier`'s rules, and accepts the edges those rules accept.
#[test]
fn validate_refuses_a_body_the_backend_would_refuse() {
    type Edit = fn(&mut BoneBody);
    let with = |edit: Edit| {
        let mut skeleton = chain(2, |_| true);
        let body = skeleton.bones[1].body.as_mut().unwrap();
        edit(body);
        skeleton
    };
    let refused: [(Edit, &str); 17] = [
        (|b| b.collider.shape = Shape::sphere(s(0.0)), "zero or negative extent"),
        (|b| b.collider.shape = Shape::sphere(s(-0.1)), "zero or negative extent"),
        // `capsule(half_height, radius)`: a zero radius, then a negative half height.
        (|b| b.collider.shape = Shape::capsule(s(0.1), s(0.0)), "zero or negative extent"),
        (|b| b.collider.shape = Shape::capsule(s(-0.1), s(0.1)), "zero or negative extent"),
        (|b| b.collider.shape = Shape::cuboid(v(0.1, 0.0, 0.1)), "zero or negative extent"),
        (|b| b.collider.shape = Shape::cuboid(v(0.1, 0.1, -0.1)), "zero or negative extent"),
        (
            |b| b.collider.rotation = Quat::new(s(0.0), s(0.0), s(0.0), s(2.0)),
            "rotation is not a unit quaternion",
        ),
        (|b| b.body.mass = s(-1.0), "mass or damping is negative"),
        (|b| b.body.linear_damping = s(-0.5), "mass or damping is negative"),
        (|b| b.body.angular_damping = s(-0.5), "mass or damping is negative"),
        (|b| b.body.locked_axes = 1 << 6, "unknown bit"),
        (|b| b.collider.friction = s(-0.1), "is negative"),
        (|b| b.collider.restitution = s(-0.1), "is negative"),
        (|b| b.collider.density = s(-1.0), "is negative"),
        (|b| b.collider.impact_threshold = s(-1.0), "is negative"),
        (|b| b.joint.as_mut().unwrap().limits = Some((s(0.5), s(-0.5))), "lower limit"),
        (|b| b.collider.shape.radius = s(f64::NAN), "not finite"),
    ];
    for (index, (edit, message)) in refused.into_iter().enumerate() {
        let error = with(edit).validate().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "case {index}: {error}");
        assert!(error.message().contains(message), "case {index}: {error}");
        assert!(error.message().contains("bone 1"), "case {index}: {error}");
    }
    // A zero axis is refused on the joints that read it and accepted on those that do not.
    for (kind, accepted) in [
        (JointKind::Revolute, false),
        (JointKind::Prismatic, false),
        (JointKind::Spherical, true),
        (JointKind::Fixed, true),
    ] {
        let mut skeleton = chain(2, |_| true);
        let joint = skeleton.bones[1].body.as_mut().unwrap().joint.as_mut().unwrap();
        (joint.kind, joint.axis) = (kind, Vec3::ZERO);
        let result = skeleton.validate();
        assert_eq!(result.is_ok(), accepted, "{kind:?}: {result:?}");
        if let Err(error) = result {
            assert!(error.message().contains("the joint's axis is zero"), "{error}");
        }
    }
    // The edges the rules accept: no mass, a capsule of zero half height (a sphere), every
    // coefficient zero, every axis locked, equal limits.
    let accepted: [Edit; 5] = [
        |b| b.body.mass = s(0.0),
        |b| b.collider.shape = Shape::capsule(s(0.0), s(0.1)),
        |b| {
            b.collider.friction = s(0.0);
            b.collider.restitution = s(0.0);
            b.collider.density = s(0.0);
            b.collider.impact_threshold = s(0.0);
        },
        |b| b.body.locked_axes = locked_axes::TRANSLATION | locked_axes::ROTATION,
        |b| b.joint.as_mut().unwrap().limits = Some((s(0.25), s(0.25))),
    ];
    for (index, edit) in accepted.into_iter().enumerate() {
        assert!(with(edit).validate().is_ok(), "accepted case {index}");
    }
}

/// `q` scaled so that its squared length is `1 + excess`.
fn off_unit(q: Quat, excess: f64) -> Quat {
    let k = s((1.0 + excess).sqrt());
    Quat::new(q.x * k, q.y * k, q.z * k, q.w * k)
}

/// The test backend's unit test on a rotation, which physics-rapier's matches.
fn unit(q: Quat) -> bool {
    (q.length_squared() - Scalar::ONE).abs() < s(1e-6)
}

/// Every rest and clip rotation within the unit tolerance on its own, so both `validate`s
/// pass, but their product down a 4-bone chain past it: every bone row still gets a body
/// the test backend accepts, created once, and every sweep is one it applies.
#[test]
fn a_chain_of_rotations_each_within_the_tolerance_gets_every_body() {
    const EXCESS: f64 = 0.9e-6;
    let mut skeleton = chain(BONES, |_| true);
    for bone in &mut skeleton.bones {
        bone.rest.rotation = off_unit(bone.rest.rotation, EXCESS);
    }
    let mut walk = clip(BONES as u8, 4, 4, true, 0.0);
    for sample in &mut walk.samples {
        sample.rotation = off_unit(sample.rotation, EXCESS);
    }
    skeleton.validate().unwrap();
    walk.validate().unwrap();
    // The premise: the composed rotation of the deepest bone is past the tolerance.
    let rest = SimPose::rest(&skeleton);
    assert!(!unit(rest.bones()[BONES - 1].rotation), "the chain stays within the tolerance");
    let machine = StateMachine {
        states: vec![state("walk", LOOP_CLIP, 1.0)],
        transitions: Vec::new(),
        initial: 0,
    };
    let library =
        Library::new().with(SKELETON, skeleton).with(MACHINE, machine).with(LOOP_CLIP, walk);
    let mut options = Options::new(WorldKind::Server);
    options.physics = true;
    let mut fixture = options.build();
    fixture.assets(library.pin());
    fixture.spawn_at(key(HERO), AnimRig::new(MACHINE, SKELETON), root());
    fixture.world_mut().service_mut::<Ragdoll>().unwrap().actions =
        vec![(2, key(HERO), RagdollAction::Spawn)];
    fixture.run(24);
    let mut created = Vec::new();
    let mut swept = 0;
    for call in fixture.calls.take() {
        match call {
            Call::CreateBody { key, transform, .. } => {
                assert!(unit(transform.rotation), "{key:?} created at {transform:?}");
                created.push(key);
            }
            Call::SetTransform(key, transform) => {
                assert!(unit(transform.rotation), "{key:?} swept to {transform:?}");
                swept += 1;
            }
            _ => {}
        }
    }
    // The hero's root has no body, so the creates are the bone rows', each once: a refused
    // create would be retried at every sync.
    let bones = bones_of(&fixture, HERO);
    let mut expected: Vec<StableKey> =
        (0..BONES as u8).map(|bone| bones.key(bone).unwrap()).collect();
    expected.sort_unstable();
    created.sort_unstable();
    assert_eq!(created, expected);
    assert!(swept > 0, "the animated bones are swept");
}
