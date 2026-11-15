//! The skeleton processor: a sim skeleton from the nodes of a glTF source marked under
//! `extras.lactose` (`README.md`, "The source schema" and "Choosing the sim bones").

use std::collections::BTreeMap;

use lactose_anim::{
    Bone, BoneBody, BoneJoint, BoneKey, BoneRole, MAX_SIM_BONES, Skeleton, SkeletonLoader, Socket,
};
use lactose_asset_api::{CookedAsset, Loader, Processor};
use lactose_core::{Error, ErrorKind, Result};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Collider, CollisionGroups, JointKind, RigidBody, Shape, locked_axes};
use serde::Deserialize;

use crate::codec;
use crate::gltf::{Document, invalid};
use crate::json::{self, MARK};
use crate::space::{Pose, to_engine_frame};

/// The [`Processor`] of [`Skeleton`]s: the JSON of a `.gltf` file in, the skeleton byte form
/// of `crates/anim/README.md`, "Assets", out, at the build's scalar width, with no
/// dependencies.
///
/// The nodes marked as bones become the bones, in the order a walk of the node tree meets
/// them, each with the rest pose of the nodes between it and its parent bone composed; the
/// root bone's rest is turned half a turn about `+Y` into the engine's frame. The nodes
/// marked as sockets become the sockets. What it writes is loaded back through
/// [`SkeletonLoader`] before it is returned, so a skeleton the runtime would refuse is never
/// cooked. Every refusal is `InvalidData` naming the node, its index and name, except a
/// binary glTF, which is `Unsupported`.
#[derive(Clone, Copy, Default, Debug)]
pub struct SkeletonProcessor;

impl Processor for SkeletonProcessor {
    type Asset = Skeleton;

    fn process(&self, source: &[u8]) -> Result<CookedAsset> {
        let skeleton = read(source)?;
        let bytes = codec::encode_skeleton(&skeleton);
        let loaded = SkeletonLoader.load(&bytes, &[])?;
        assert_eq!(loaded, skeleton, "the encoder and anim's decoder disagree on the byte form");
        Ok(CookedAsset::new::<Skeleton>(bytes, []))
    }
}

/// The skeleton a source describes, validated.
fn read(source: &[u8]) -> Result<Skeleton> {
    // The magic of the binary container (glTF 2.0, section 4.4.2).
    if source.starts_with(b"glTF") {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "a binary glTF (`.glb`): the cooker reads the JSON form, `.gltf`",
        ));
    }
    let document = Document::parse(source)?;
    let hierarchy = document.hierarchy()?;
    let mut bones: Vec<(usize, Bone)> = Vec::new();
    let mut sockets: Vec<(usize, Socket)> = Vec::new();
    // The node of each socket key, so a repeated key costs one lookup.
    let mut socket_keys = BTreeMap::new();
    // Each node's place, filled in preorder: a parent's before its children's.
    let mut places: Vec<Option<Place>> = vec![None; document.nodes.len()];
    for &node in &hierarchy.preorder {
        let parent = hierarchy.parent(node).map(|parent| places[parent].expect("a parent first"));
        let above = parent.and_then(|parent| parent.bone);
        let pose = parent.map_or(Ok(Pose::IDENTITY), |parent| parent.pose).and_then(|pose| {
            document.local(node).map(|local| pose.compose(local)).map_err(|_| node)
        });
        let mut place = Place { bone: above, pose };
        if let Some(mark) = mark(&document, node)? {
            let Some(name) = document.nodes[node].name.as_deref() else {
                return Err(
                    document.refuse(node, "marked, with no name: its key is its name's hash")
                );
            };
            let key = BoneKey::from_name(name);
            match mark {
                Mark::Bone(source) => {
                    if !document.in_rig(node)? {
                        return Err(document.refuse(node, "a bone that is no joint of the skins"));
                    }
                    if let (None, Some(&(root, _))) = (above, bones.first()) {
                        return Err(document.refuse(
                            node,
                            format!("a second root bone, beside {}", document.label(root)),
                        ));
                    }
                    if bones.len() == MAX_SIM_BONES {
                        return Err(document.refuse(
                            node,
                            format!(
                                "bone {}: a sim skeleton has at most {MAX_SIM_BONES}",
                                bones.len() + 1
                            ),
                        ));
                    }
                    if let Some(&(other, _)) = bones.iter().find(|(_, bone)| bone.key == key) {
                        return Err(document
                            .refuse(node, format!("the bone key of {}", document.label(other))));
                    }
                    let rest = place.pose(&document)?;
                    let rest = if above.is_none() { to_engine_frame(rest) } else { rest };
                    let index = u8::try_from(bones.len()).expect("at most MAX_SIM_BONES bones");
                    let bone = Bone {
                        key,
                        parent: above.map(|(_, index)| index),
                        role: source.role.map_or(BoneRole::Other, RoleSource::role),
                        rest: rest.to_bone_pose(),
                        body: source.body.map(BodySource::body),
                    };
                    bones.push((node, bone));
                    place = Place { bone: Some((node, index)), pose: Ok(Pose::IDENTITY) };
                }
                Mark::Socket(SocketSource {}) => {
                    let Some((_, bone)) = above else {
                        return Err(document.refuse(node, "a socket under no bone"));
                    };
                    if let Some(&other) = socket_keys.get(&key) {
                        return Err(document
                            .refuse(node, format!("the socket key of {}", document.label(other))));
                    }
                    socket_keys.insert(key, node);
                    let offset = place.pose(&document)?;
                    sockets.push((node, Socket { key, bone, offset: offset.to_bone_pose() }));
                }
            }
        }
        places[node] = Some(place);
    }
    if bones.is_empty() {
        return Err(invalid(format!("no node is marked as a bone under `extras.{MARK}`")));
    }
    let (bone_nodes, bones): (Vec<_>, Vec<_>) = bones.into_iter().unzip();
    let (socket_nodes, sockets): (Vec<_>, Vec<_>) = sockets.into_iter().unzip();
    let skeleton = Skeleton { bones, sockets };
    validate(&document, &skeleton, &bone_nodes, &socket_nodes)?;
    Ok(skeleton)
}

/// Where a node is, to the skeleton: the nearest bone at or above it, as its node and bone
/// index, and the node's pose in that bone's space (the scene's, under no bone), the
/// transforms of the nodes between composed parent first; or else the first of those nodes,
/// from the top, whose transform is refused. The refusal is raised only where a bone or a
/// socket needs the pose, so a scaled mesh beside the rig is no refusal.
#[derive(Clone, Copy)]
struct Place {
    bone: Option<(usize, u8)>,
    pose: core::result::Result<Pose, usize>,
}

impl Place {
    /// The pose, or the refusal of the node that broke it.
    fn pose(self, document: &Document) -> Result<Pose> {
        self.pose.map_err(|node| document.local(node).expect_err("a node refused before"))
    }
}

/// `node`'s mark, if its `extras` is an object holding one under [`MARK`]: refused if a key
/// of it is written twice in one object.
fn mark(document: &Document, node: usize) -> Result<Option<Mark>> {
    let extras = &document.nodes[node].extras;
    if let Some(path) = &extras.repeated {
        return Err(document.refuse(node, format!("`extras.{path}` is written twice")));
    }
    let Some(value) = &extras.mark else { return Ok(None) };
    Mark::deserialize(value)
        .map(Some)
        .map_err(|error| document.refuse(node, format!("`extras.{MARK}`: {error}")))
}

/// `Skeleton::validate`, its refusal naming the node of the first bone, or else the first
/// socket, whose addition makes the skeleton invalid.
fn validate(
    document: &Document,
    skeleton: &Skeleton,
    bone_nodes: &[usize],
    socket_nodes: &[usize],
) -> Result<()> {
    let Err(error) = skeleton.validate() else { return Ok(()) };
    let prefix = |bones: usize, sockets: usize| Skeleton {
        bones: skeleton.bones[..bones].to_vec(),
        sockets: skeleton.sockets[..sockets].to_vec(),
    };
    for bones in 1..=skeleton.bones.len() {
        if let Err(error) = prefix(bones, 0).validate() {
            return Err(document.refuse(bone_nodes[bones - 1], error.message()));
        }
    }
    // The bones are valid and each socket is checked on its own, so once a prefix of the
    // sockets fails every longer one does: the shortest is found by bisection, a few
    // validations whatever the count.
    let (mut valid, mut failing) = (0, skeleton.sockets.len());
    while failing - valid > 1 {
        let middle = valid + (failing - valid) / 2;
        if prefix(skeleton.bones.len(), middle).validate().is_ok() {
            valid = middle;
        } else {
            failing = middle;
        }
    }
    match prefix(skeleton.bones.len(), failing).validate() {
        Err(error) if failing > 0 => {
            Err(document.refuse(socket_nodes[failing - 1], error.message()))
        }
        _ => Err(error),
    }
}

// ---- The source schema: `README.md`, "The source schema" ----

/// What a node is to the skeleton. Every struct of the schema is written as a JSON object
/// and as nothing else (`json::object`).
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mark {
    // Boxed: a body is a few hundred bytes, a socket mark none.
    #[serde(deserialize_with = "json::boxed")]
    Bone(Box<BoneSource>),
    #[serde(deserialize_with = "json::object")]
    Socket(SocketSource),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoneSource {
    #[serde(default, deserialize_with = "json::optional_name")]
    role: Option<RoleSource>,
    #[serde(default, deserialize_with = "json::optional")]
    body: Option<BodySource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SocketSource {}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoleSource {
    Hips,
    Spine,
    Head,
    HandLeft,
    HandRight,
    FootLeft,
    FootRight,
}

impl RoleSource {
    fn role(self) -> BoneRole {
        match self {
            Self::Hips => BoneRole::Hips,
            Self::Spine => BoneRole::Spine,
            Self::Head => BoneRole::Head,
            Self::HandLeft => BoneRole::HandLeft,
            Self::HandRight => BoneRole::HandRight,
            Self::FootLeft => BoneRole::FootLeft,
            Self::FootRight => BoneRole::FootRight,
        }
    }
}

/// A bone's body: the `RigidBody` of its type, whose fields a source may override, a
/// collider, and an optional joint.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BodySource {
    #[serde(default, deserialize_with = "json::optional_name")]
    body_type: Option<BodyTypeSource>,
    mass: Option<f64>,
    linear_damping: Option<f64>,
    angular_damping: Option<f64>,
    gravity_scale: Option<f64>,
    #[serde(default, deserialize_with = "json::optional_names")]
    locked_axes: Option<Vec<AxisSource>>,
    can_sleep: Option<bool>,
    #[serde(deserialize_with = "json::object")]
    collider: ColliderSource,
    #[serde(default, deserialize_with = "json::optional")]
    joint: Option<JointSource>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BodyTypeSource {
    Dynamic,
    Kinematic,
    Fixed,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AxisSource {
    TranslationX,
    TranslationY,
    TranslationZ,
    RotationX,
    RotationY,
    RotationZ,
}

/// A collider in the bone's space. `membership` and `filter` have no default: the
/// `Collider::new` default, every group, is one a bone's collider never has (the bone
/// collider rule of `crates/anim/README.md`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ColliderSource {
    #[serde(deserialize_with = "json::object")]
    shape: ShapeSource,
    membership: u32,
    filter: u32,
    offset: Option<[f64; 3]>,
    rotation: Option<[f64; 4]>,
    friction: Option<f64>,
    restitution: Option<f64>,
    density: Option<f64>,
    impact_threshold: Option<f64>,
    sensor: Option<bool>,
    events: Option<bool>,
    material: Option<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum ShapeSource {
    #[serde(deserialize_with = "json::object")]
    Sphere(SphereSource),
    #[serde(deserialize_with = "json::object")]
    Capsule(CapsuleSource),
    #[serde(deserialize_with = "json::object")]
    Cuboid(CuboidSource),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SphereSource {
    radius: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapsuleSource {
    half_height: f64,
    radius: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CuboidSource {
    half_extents: [f64; 3],
}

/// The joint to the nearest ancestor bone with a body: the anchors in that ancestor's
/// space and in the bone's, the axis in the ancestor's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JointSource {
    #[serde(deserialize_with = "json::name")]
    kind: JointKindSource,
    anchor_parent: Option<[f64; 3]>,
    anchor_bone: Option<[f64; 3]>,
    axis: Option<[f64; 3]>,
    limits: Option<[f64; 2]>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JointKindSource {
    Fixed,
    Spherical,
    Revolute,
    Prismatic,
}

fn scalar(value: f64) -> Scalar {
    Scalar::from_f64(value)
}

fn vec3([x, y, z]: [f64; 3]) -> Vec3 {
    Vec3::new(scalar(x), scalar(y), scalar(z))
}

impl BodySource {
    fn body(self) -> BoneBody {
        let mut body = match self.body_type.unwrap_or(BodyTypeSource::Dynamic) {
            BodyTypeSource::Dynamic => RigidBody::dynamic(),
            BodyTypeSource::Kinematic => RigidBody::kinematic(),
            BodyTypeSource::Fixed => RigidBody::fixed(),
        };
        let set = |field: &mut Scalar, value: Option<f64>| {
            if let Some(value) = value {
                *field = scalar(value);
            }
        };
        set(&mut body.mass, self.mass);
        set(&mut body.linear_damping, self.linear_damping);
        set(&mut body.angular_damping, self.angular_damping);
        set(&mut body.gravity_scale, self.gravity_scale);
        if let Some(axes) = self.locked_axes {
            body.locked_axes = axes.iter().fold(locked_axes::NONE, |bits, axis| {
                bits | match axis {
                    AxisSource::TranslationX => locked_axes::TRANSLATION_X,
                    AxisSource::TranslationY => locked_axes::TRANSLATION_Y,
                    AxisSource::TranslationZ => locked_axes::TRANSLATION_Z,
                    AxisSource::RotationX => locked_axes::ROTATION_X,
                    AxisSource::RotationY => locked_axes::ROTATION_Y,
                    AxisSource::RotationZ => locked_axes::ROTATION_Z,
                }
            });
        }
        if let Some(can_sleep) = self.can_sleep {
            body.can_sleep = can_sleep;
        }
        BoneBody {
            body,
            collider: self.collider.collider(),
            joint: self.joint.map(JointSource::joint),
        }
    }
}

impl ColliderSource {
    fn collider(self) -> Collider {
        let shape = match self.shape {
            ShapeSource::Sphere(SphereSource { radius }) => Shape::sphere(scalar(radius)),
            ShapeSource::Capsule(CapsuleSource { half_height, radius }) => {
                Shape::capsule(scalar(half_height), scalar(radius))
            }
            ShapeSource::Cuboid(CuboidSource { half_extents }) => Shape::cuboid(vec3(half_extents)),
        };
        let mut collider =
            Collider::new(shape).with_groups(CollisionGroups::new(self.membership, self.filter));
        if let Some(offset) = self.offset {
            collider.offset = vec3(offset);
        }
        if let Some([x, y, z, w]) = self.rotation {
            collider.rotation = Quat::new(scalar(x), scalar(y), scalar(z), scalar(w));
        }
        let set = |field: &mut Scalar, value: Option<f64>| {
            if let Some(value) = value {
                *field = scalar(value);
            }
        };
        set(&mut collider.friction, self.friction);
        set(&mut collider.restitution, self.restitution);
        set(&mut collider.density, self.density);
        set(&mut collider.impact_threshold, self.impact_threshold);
        collider.sensor = self.sensor.unwrap_or(collider.sensor);
        collider.events = self.events.unwrap_or(collider.events);
        collider.material = self.material.unwrap_or(collider.material);
        collider
    }
}

impl JointSource {
    fn joint(self) -> BoneJoint {
        BoneJoint {
            kind: match self.kind {
                JointKindSource::Fixed => JointKind::Fixed,
                JointKindSource::Spherical => JointKind::Spherical,
                JointKindSource::Revolute => JointKind::Revolute,
                JointKindSource::Prismatic => JointKind::Prismatic,
            },
            anchor_parent: self.anchor_parent.map_or(Vec3::ZERO, vec3),
            anchor_bone: self.anchor_bone.map_or(Vec3::ZERO, vec3),
            // `Joint::new`'s axis, local y.
            axis: self.axis.map_or(Vec3::Y, vec3),
            limits: self.limits.map(|[min, max]| (scalar(min), scalar(max))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_json(json: &str) -> Result<Skeleton> {
        read(json.as_bytes())
    }

    /// A one-bone source whose bone's mark is `mark`.
    fn one_bone(mark: &str) -> String {
        format!(
            r#"{{"asset": {{"version": "2.0"}},
                "nodes": [{{"name": "hips", "extras": {{"lactose": {mark}}}}}]}}"#
        )
    }

    #[test]
    fn a_bare_bone_mark_is_a_bone_of_role_other_without_a_body() {
        let skeleton = read_json(&one_bone(r#"{"bone": {}}"#)).unwrap();
        assert_eq!(skeleton.bones.len(), 1);
        assert_eq!(skeleton.bones[0].role, BoneRole::Other);
        assert_eq!(skeleton.bones[0].body, None);
        assert_eq!(skeleton.bones[0].key, BoneKey::from_name("hips"));
    }

    #[test]
    fn an_omitted_body_field_keeps_the_physics_default() {
        let skeleton = read_json(&one_bone(
            r#"{"bone": {"body": {"collider": {"shape": {"sphere": {"radius": 0.25}},
                                               "membership": 1, "filter": 2}}}}"#,
        ))
        .unwrap();
        let body = skeleton.bones[0].body.unwrap();
        assert_eq!(body.body, RigidBody::dynamic());
        let expected =
            Collider::new(Shape::sphere(scalar(0.25))).with_groups(CollisionGroups::new(1, 2));
        assert_eq!(body.collider, expected);
        assert_eq!(body.joint, None);
    }

    #[test]
    fn a_mark_is_one_of_bone_or_socket_with_no_unknown_field() {
        for (mark, says) in [
            (r#"{"bone": {}, "socket": {}}"#, "map with a single key"),
            (r#"{"bone": {"rol": "hips"}}"#, "unknown field `rol`"),
            (r#"{"bone": {"role": "tail"}}"#, "unknown variant `tail`"),
            (r#"{"socket": {"bone": 0}}"#, "unknown field `bone`"),
            (r#"{"joint": {}}"#, "unknown variant `joint`"),
            (
                r#"{"bone": {"body": {"collider": {"shape": {"sphere": {"radius": 1, "r": 1}},
                                                   "membership": 1, "filter": 2}}}}"#,
                "unknown field `r`",
            ),
            (
                r#"{"bone": {"body": {"collider": {"shape": {"sphere": {"radius": 1}}}}}}"#,
                "missing field `membership`",
            ),
            // A name written as a one-key object, which serde reads as a unit variant: each
            // of the four fields a name is read in.
            (r#"{"bone": {"role": {"hips": null}}}"#, "expected a string"),
            (
                r#"{"bone": {"body": {"body_type": {"fixed": null},
                    "collider": {"shape": {"sphere": {"radius": 1}}, "membership": 1, "filter": 2}}}}"#,
                "expected a string",
            ),
            (
                r#"{"bone": {"body": {"locked_axes": [{"rotation_x": null}],
                    "collider": {"shape": {"sphere": {"radius": 1}}, "membership": 1, "filter": 2}}}}"#,
                "expected a string",
            ),
            (
                r#"{"bone": {"body": {"joint": {"kind": {"spherical": null}},
                    "collider": {"shape": {"sphere": {"radius": 1}}, "membership": 1, "filter": 2}}}}"#,
                "expected a string",
            ),
        ] {
            let error = read_json(&one_bone(mark)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{mark}");
            assert!(error.message().starts_with("node 0 `hips`: "), "{mark}: {error}");
            assert!(error.message().contains(says), "{mark}: {error}");
        }
    }

    #[test]
    fn a_validate_refusal_names_the_node_of_the_bone_it_names() {
        // Two hips: the second bone, node 1, is the one validate refuses.
        let json = r#"{"asset": {"version": "2.0"}, "nodes": [
            {"name": "a", "children": [1], "extras": {"lactose": {"bone": {"role": "hips"}}}},
            {"name": "b", "extras": {"lactose": {"bone": {"role": "hips"}}}}]}"#;
        let error = read_json(json).unwrap_err();
        assert_eq!(error.message(), "node 1 `b`: bone 1: another bone has its role");
    }
}
