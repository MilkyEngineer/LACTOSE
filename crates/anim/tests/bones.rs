//! Bones and ragdolls (`README.md`, "Bones and ragdolls"; "Proof for milestone 1", the
//! bones row): the rows `Bones::spawn` records and nothing on a second call; the three
//! `Stage::Post` systems in a server world and not in a prediction world; the kinematic
//! bodies swept to the previous tick's pose as the physics test backend sees it; `set_limp`
//! both ways; `report_bones` from limp rows alone; the collider rule `Skeleton::validate`
//! checks; and no command for a character without bones. The fit itself is in
//! `tests/evaluation.rs`.

mod common;

use common::fake::Call;
use common::{
    Fixture, Library, MACHINE, Options, Ragdoll, RagdollAction, SKELETON, bone_body, chain, clip,
    clip_ref, key, state, v,
};
use lactose_anim::{AnimRig, BoneLink, Bones, Clip, SimPose, Skeleton, StateMachine};
use lactose_asset_api::AssetRef;
use lactose_core::{ErrorKind, StableKey};
use lactose_ecs::{ClaimedBy, OwnedBy, WorldKind};

use lactose_physics_api::{BodyType, Collider, CollisionGroups, Joint, RigidBody, Transform};

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
        let expected = pose.bones()[usize::from(bone)].in_world(&root());
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
            .map(|&bone| (bones.key(bone).unwrap(), pose.world(&root, bone).unwrap()))
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
