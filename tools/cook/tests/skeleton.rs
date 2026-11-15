//! The committed knight source cooked by the `ricotta` binary: the pack mounts in a store,
//! the skeleton loads through `SkeletonLoader` and pins, and it is the skeleton the source
//! describes, turned half a turn about `+Y`. Cooking twice, and cooking the same sources
//! written in another order, writes the same bytes.

#![forbid(unsafe_code)]

mod common;

use common::{Env, knight, run_binary, sources};
use lactose_anim::{
    Bone, BoneBody, BoneJoint, BoneKey, BonePose, BoneRole, SimPose, Skeleton, SkeletonLoader,
    Socket,
};
use lactose_asset::Store;
use lactose_asset_api::{
    AssetCatalog, AssetId, AssetRef, AssetSet, ErasedLoader, LoadPriority, PinnedTable,
};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Collider, CollisionGroups, JointKind, RigidBody, Shape, locked_axes};

fn s(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(s(x), s(y), s(z))
}

fn q(x: f64, y: f64, z: f64, w: f64) -> Quat {
    Quat::new(s(x), s(y), s(z), s(w))
}

/// The skeleton `tests/sources/characters/knight.gltf` describes, worked out by hand from
/// its nodes (`README.md`, "Choosing the sim bones").
fn expected() -> Skeleton {
    // `Armature` (0, 0, 0.5) then `hips` (0, 1, 0): the root, at +Z in the source, turned
    // half a turn about +Y, so at -Z and facing the other way.
    let mut hips_body = RigidBody::dynamic();
    hips_body.mass = s(8.0);
    hips_body.linear_damping = s(0.5);
    hips_body.locked_axes = locked_axes::ROTATION_X | locked_axes::ROTATION_Z;
    let hips = Bone {
        key: BoneKey::from_name("hips"),
        parent: None,
        role: BoneRole::Hips,
        rest: BonePose::new(v(0.0, 1.0, -0.5), q(0.0, 1.0, 0.0, 0.0)),
        body: Some(BoneBody {
            body: hips_body,
            collider: Collider::new(Shape::capsule(s(0.25), s(0.125)))
                .with_groups(CollisionGroups::new(1, 2)),
            joint: None,
        }),
    };
    // `spine_twist` (0, 0.25, 0) turned half about y, then `chest` (0, 0.125, 0.5): the
    // unmarked node composed into the chest's parent-local rest, and not turned again.
    let chest = Bone {
        key: BoneKey::from_name("chest"),
        parent: Some(0),
        role: BoneRole::Spine,
        rest: BonePose::new(v(0.0, 0.375, -0.5), q(0.0, 1.0, 0.0, 0.0)),
        body: Some(BoneBody {
            body: RigidBody::dynamic(),
            collider: Collider::new(Shape::sphere(s(0.25))).with_groups(CollisionGroups::new(1, 2)),
            joint: Some(BoneJoint {
                kind: JointKind::Spherical,
                anchor_parent: v(0.0, 0.375, -0.5),
                anchor_bone: Vec3::ZERO,
                axis: Vec3::Y,
                limits: None,
            }),
        }),
    };
    // A matrix of a half turn about y and a translation.
    let head = Bone {
        key: BoneKey::from_name("head"),
        parent: Some(1),
        role: BoneRole::Head,
        rest: BonePose::new(v(0.0, 0.25, 0.0), q(0.0, 1.0, 0.0, 0.0)),
        body: None,
    };
    let mut hand_body = RigidBody::kinematic();
    hand_body.can_sleep = false;
    let mut hand_collider = Collider::new(Shape::cuboid(v(0.0625, 0.125, 0.03125)))
        .with_groups(CollisionGroups::new(1, 4))
        .with_offset(v(0.0, 0.0625, 0.0))
        .with_material(7);
    hand_collider.friction = s(1.0);
    let hand = Bone {
        key: BoneKey::from_name("hand_r"),
        parent: Some(1),
        role: BoneRole::HandRight,
        rest: BonePose::new(v(0.25, 0.0, 0.0), q(0.5, 0.5, 0.5, 0.5)),
        body: Some(BoneBody {
            body: hand_body,
            collider: hand_collider,
            joint: Some(BoneJoint {
                kind: JointKind::Revolute,
                anchor_parent: v(0.25, 0.0, 0.0),
                anchor_bone: Vec3::ZERO,
                axis: v(1.0, 0.0, 0.0),
                limits: Some((s(-0.5), s(1.0))),
            }),
        }),
    };
    Skeleton {
        bones: vec![hips, chest, head, hand],
        sockets: vec![Socket {
            key: BoneKey::from_name("grip"),
            bone: 3,
            offset: BonePose::new(v(0.0, -0.125, 0.25), Quat::IDENTITY),
        }],
    }
}

/// The skeleton of the pack at `pack`, loaded by a store that mounted it alone, resolved
/// and pinned as a region pins its assets.
fn load(env: &Env, pack: &std::path::Path, id: AssetId) -> Skeleton {
    let loaders = vec![ErasedLoader::new(SkeletonLoader)];
    let store = Store::new(env.executor.clone(), env.io.clone(), loaders).unwrap();
    env.wait(store.mount(pack)).expect("the pack mounts");
    store.request_newest(id, LoadPriority(0));
    env.wait(store.settled());
    let set: AssetSet = [id].into_iter().collect();
    let table = store.resolve(&set).ready().expect("the skeleton resolves");
    let pinned = PinnedTable::pin(&store, &table).ready().expect("the skeleton pins");
    pinned.get(AssetRef::<Skeleton>::new(id)).expect("a skeleton").clone()
}

#[test]
fn the_knight_cooks_mounts_loads_pins_and_is_the_skeleton_its_nodes_describe() {
    let env = Env::new("knight");
    let pack = env.dir.join("out").join("knight.pack");
    let printed = run_binary(&sources(), &pack);
    let id = AssetId::from_name("characters/knight.skeleton");
    let fields: Vec<&str> = printed.split_whitespace().collect();
    assert_eq!(fields[..2], ["characters/knight.skeleton", &format!("{:#018x}", id.to_bits())]);
    assert_eq!(fields.len(), 3, "one asset: {printed}");

    let skeleton = load(&env, &pack, id);
    assert_eq!(skeleton, expected());
    // More nodes than bones: nine nodes, four bones, one socket.
    assert_eq!((skeleton.bones.len(), skeleton.sockets.len()), (4, 1));

    // The half turn, applied to the root alone, turns the whole skeleton: the hips sit at
    // +Z (0.5) in the source's scene and at -Z once cooked, and the hand, at -X (-0.25) in
    // the scene, is at +X in the root's space, its parent-local rest as authored.
    let rest = SimPose::rest(&skeleton);
    assert_eq!(rest.bone(0).unwrap().translation, v(0.0, 1.0, -0.5));
    assert_eq!(rest.bone(3).unwrap().translation, v(0.25, 1.375, 0.0));
}

#[test]
fn cooking_twice_writes_the_same_bytes() {
    let env = Env::new("twice");
    let first = env.dir.join("first").join("knight.pack");
    let second = env.dir.join("second").join("knight.pack");
    let printed = [run_binary(&sources(), &first), run_binary(&sources(), &second)];
    assert_eq!(printed[0], printed[1], "the same names, IDs and versions");
    assert_eq!(env.read(&first), env.read(&second));
}

#[test]
fn sources_written_in_another_order_cook_to_the_same_bytes() {
    // Three sources, among them a directory and a file that sort differently by path and
    // by name (`a/` and `a.b.gltf`), written to two trees in opposite orders.
    let env = Env::new("order");
    let knight = knight();
    let mut flipped = knight.clone();
    let at = flipped.windows(7).position(|w| w == b"[0, 1, ").expect("the hips' translation");
    flipped[at..at + 7].copy_from_slice(b"[0, 2, ");
    let files: [(&str, &[u8]); 3] =
        [("a/knight.gltf", &knight), ("a.b.gltf", &flipped), ("z/y/knight.gltf", &knight)];
    let forward = env.dir.join("forward");
    let backward = env.dir.join("backward");
    env.write_tree(&forward, &files);
    let mut reversed = files;
    reversed.reverse();
    env.write_tree(&backward, &reversed);
    let printed = [
        run_binary(&forward, &env.dir.join("f.pack")),
        run_binary(&backward, &env.dir.join("b.pack")),
    ];
    assert_eq!(printed[0], printed[1]);
    let names: Vec<&str> = printed[0].lines().map(|line| line.split(' ').next().unwrap()).collect();
    assert_eq!(names, ["a.b.skeleton", "a/knight.skeleton", "z/y/knight.skeleton"]);
    assert_eq!(env.read(&env.dir.join("f.pack")), env.read(&env.dir.join("b.pack")));
}
