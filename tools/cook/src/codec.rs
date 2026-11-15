//! The cooker's encoder of the skeleton byte form, written from the statement in
//! `crates/anim/README.md`, "Assets": little-endian, every field at its fixed width in the
//! listed order with no padding, a scalar its canonical bits at the build's width, a list a
//! `u32` count before its elements, after a preamble of the format version and the scalar
//! width. `lactose-anim`'s decoder is the other half; the cooker loads what it wrote through
//! `SkeletonLoader` before it publishes it.

use lactose_anim::{BoneBody, BonePose, BoneRole, Skeleton};
use lactose_asset_api::Asset;
use lactose_math::{Canonical, Quat, Scalar, Vec3};
use lactose_physics_api::{BodyType, JointKind, ShapeKind};

/// The skeleton's bytes, in `Skeleton::FORMAT_VERSION` at the build's scalar width.
pub(crate) fn encode_skeleton(skeleton: &Skeleton) -> Vec<u8> {
    let mut w = Writer::open(Skeleton::FORMAT_VERSION);
    w.count(skeleton.bones.len());
    for bone in &skeleton.bones {
        w.u64(bone.key.0);
        w.u8(bone.parent.unwrap_or(u8::MAX));
        w.u8(role(bone.role));
        w.pose(&bone.rest);
        w.bool(bone.body.is_some());
        if let Some(body) = &bone.body {
            w.body(body);
        }
    }
    w.count(skeleton.sockets.len());
    for socket in &skeleton.sockets {
        w.u64(socket.key.0);
        w.u8(socket.bone);
        w.pose(&socket.offset);
    }
    w.0
}

/// A role's byte: its place in `BoneRole`'s declaration, from zero.
fn role(role: BoneRole) -> u8 {
    match role {
        BoneRole::Other => 0,
        BoneRole::Hips => 1,
        BoneRole::Spine => 2,
        BoneRole::Head => 3,
        BoneRole::HandLeft => 4,
        BoneRole::HandRight => 5,
        BoneRole::FootLeft => 6,
        BoneRole::FootRight => 7,
    }
}

struct Writer(Vec<u8>);

impl Writer {
    /// The preamble: the format version, then the scalar width.
    fn open(format: u32) -> Self {
        let mut writer = Self(Vec::new());
        writer.u32(format);
        writer.u32(u32::try_from(Scalar::BYTES).expect("a scalar of a few bytes"));
        writer
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// A list's length. A skeleton holds at most `MAX_SIM_BONES` bones, and its sockets are
    /// nodes of one source, so a count past `u32` is a broken invariant.
    fn count(&mut self, count: usize) {
        self.u32(u32::try_from(count).expect("a list of fewer than 2^32 elements"));
    }

    fn scalar(&mut self, value: Scalar) {
        self.0.extend_from_slice(&value.to_canonical_bytes());
    }

    fn vec3(&mut self, value: Vec3) {
        self.0.extend_from_slice(&value.to_canonical_bytes());
    }

    fn quat(&mut self, value: Quat) {
        self.0.extend_from_slice(&value.to_canonical_bytes());
    }

    fn pose(&mut self, pose: &BonePose) {
        self.vec3(pose.translation);
        self.quat(pose.rotation);
    }

    /// A `BoneBody`: the body, the collider, then the optional joint.
    fn body(&mut self, body: &BoneBody) {
        let rigid = &body.body;
        self.scalar(rigid.mass);
        self.scalar(rigid.linear_damping);
        self.scalar(rigid.angular_damping);
        self.scalar(rigid.gravity_scale);
        self.u8(match rigid.body_type {
            BodyType::Dynamic => 0,
            BodyType::Kinematic => 1,
            BodyType::Fixed => 2,
        });
        self.u8(rigid.locked_axes);
        self.bool(rigid.can_sleep);
        let collider = &body.collider;
        self.u8(match collider.shape.kind {
            ShapeKind::Sphere => 0,
            ShapeKind::Capsule => 1,
            ShapeKind::Cuboid => 2,
        });
        self.vec3(collider.shape.half_extents);
        self.scalar(collider.shape.radius);
        self.vec3(collider.offset);
        self.quat(collider.rotation);
        self.scalar(collider.friction);
        self.scalar(collider.restitution);
        self.scalar(collider.density);
        self.scalar(collider.impact_threshold);
        self.u32(collider.groups.membership);
        self.u32(collider.groups.filter);
        self.bool(collider.sensor);
        self.bool(collider.events);
        self.u16(collider.material);
        self.bool(body.joint.is_some());
        if let Some(joint) = &body.joint {
            self.u8(match joint.kind {
                JointKind::Fixed => 0,
                JointKind::Spherical => 1,
                JointKind::Revolute => 2,
                JointKind::Prismatic => 3,
            });
            self.vec3(joint.anchor_parent);
            self.vec3(joint.anchor_bone);
            self.vec3(joint.axis);
            self.bool(joint.limits.is_some());
            if let Some((min, max)) = joint.limits {
                self.scalar(min);
                self.scalar(max);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use lactose_anim::{Bone, BoneJoint, BoneKey, Socket};
    use lactose_physics_api::{Collider, CollisionGroups, RigidBody, Shape, locked_axes};

    use super::*;

    // The golden skeleton and its bytes are copied from `crates/anim/tests/format.rs`,
    // `one_asset_of_each_kind_matches_its_golden_bytes`, at the commit this cooker was
    // written against; they were written by hand from the README's per-kind list, apart
    // from both encoders. Every two neighbouring fields of one width differ in them, so a
    // swap in this encoder fails the test.

    /// Eight-byte scalars' bits, written out: the golden bytes below are at that width.
    const F0: [u8; 8] = 0u64.to_le_bytes();
    const F1: [u8; 8] = 0x3FF0_0000_0000_0000u64.to_le_bytes();
    const F2: [u8; 8] = 0x4000_0000_0000_0000u64.to_le_bytes();
    const F_HALF: [u8; 8] = 0x3FE0_0000_0000_0000u64.to_le_bytes();
    const F_QUARTER: [u8; 8] = 0x3FD0_0000_0000_0000u64.to_le_bytes();
    const F_EIGHTH: [u8; 8] = 0x3FC0_0000_0000_0000u64.to_le_bytes();
    /// `sqrt(43) / 8` rounded, the `w` that makes `(1/8, 1/4, 1/2, w)` a unit quaternion.
    const W_BITS: u64 = 0x3FEA_3AD1_2A1D_A160;
    /// The preamble of format version one at eight-byte scalars.
    const PREAMBLE: [u8; 8] = [1, 0, 0, 0, 8, 0, 0, 0];

    fn s(value: f64) -> Scalar {
        Scalar::from_f64(value)
    }

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(s(x), s(y), s(z))
    }

    #[test]
    fn the_encoder_writes_anims_golden_skeleton_bytes() {
        assert_eq!(Scalar::BYTES, 8, "the golden bytes are at eight-byte scalars");
        // Two bones with bodies, the second a kinematic cuboid on a limited revolute joint
        // to the first, and one socket: a shape's kind before its extents.
        let mut hips = RigidBody::dynamic();
        hips.mass = s(2.0);
        hips.linear_damping = s(0.5);
        hips.angular_damping = s(0.25);
        hips.gravity_scale = s(1.0);
        hips.locked_axes = locked_axes::ROTATION_X | locked_axes::ROTATION_Z;
        hips.can_sleep = true;
        let mut hips_collider = Collider::new(Shape::sphere(s(0.5)));
        hips_collider.offset = v(0.0, 0.25, 0.0);
        hips_collider.rotation = Quat::IDENTITY;
        hips_collider.friction = s(0.5);
        hips_collider.restitution = s(0.25);
        hips_collider.density = s(1.0);
        hips_collider.impact_threshold = s(2.0);
        hips_collider.groups = CollisionGroups::new(1, 2);
        hips_collider.sensor = false;
        hips_collider.events = true;
        hips_collider.material = 3;
        let mut foot = RigidBody::kinematic();
        foot.mass = s(1.0);
        foot.linear_damping = s(0.0);
        foot.angular_damping = s(0.5);
        foot.gravity_scale = s(2.0);
        foot.locked_axes = 0;
        foot.can_sleep = false;
        let mut foot_collider = Collider::new(Shape::cuboid(v(0.5, 1.0, 2.0)));
        foot_collider.offset = v(0.25, 0.0, 0.0);
        // Four different coefficients, so a swap inside a quaternion is caught too.
        foot_collider.rotation = Quat::new(s(0.125), s(0.25), s(0.5), s(f64::from_bits(W_BITS)));
        foot_collider.friction = s(1.0);
        foot_collider.restitution = s(0.5);
        foot_collider.density = s(2.0);
        foot_collider.impact_threshold = s(0.25);
        foot_collider.groups = CollisionGroups::new(4, 8);
        foot_collider.sensor = true;
        foot_collider.events = false;
        foot_collider.material = 0x0102;
        let hinge = BoneJoint {
            kind: JointKind::Revolute,
            anchor_parent: v(0.0, 0.25, 0.0),
            anchor_bone: v(0.0, 0.0, 0.5),
            axis: v(1.0, 0.0, 0.0),
            limits: Some((s(0.25), s(1.0))),
        };
        let skeleton = Skeleton {
            bones: vec![
                Bone {
                    key: BoneKey(0x41),
                    parent: None,
                    role: BoneRole::Hips,
                    rest: BonePose { translation: v(0.0, 1.0, 0.0), rotation: Quat::IDENTITY },
                    body: Some(BoneBody { body: hips, collider: hips_collider, joint: None }),
                },
                Bone {
                    key: BoneKey(0x42),
                    parent: Some(0),
                    role: BoneRole::FootRight,
                    rest: BonePose { translation: v(0.0, 0.5, 0.0), rotation: Quat::IDENTITY },
                    body: Some(BoneBody {
                        body: foot,
                        collider: foot_collider,
                        joint: Some(hinge),
                    }),
                },
            ],
            sockets: vec![Socket { key: BoneKey(0x51), bone: 1, offset: BonePose::IDENTITY }],
        };
        let golden = [
            &PREAMBLE[..],
            &[2, 0, 0, 0], // two bones
            &[0x41, 0, 0, 0, 0, 0, 0, 0],
            &[0xFF], // no parent
            &[1],    // Hips
            &[F0, F1, F0, F0, F0, F0, F1].concat(),
            &[1], // a body
            &[F2, F_HALF, F_QUARTER, F1].concat(),
            &[0],    // dynamic
            &[0x28], // ROTATION_X | ROTATION_Z
            &[1],    // can_sleep
            &[0],    // a sphere
            &[F0, F0, F0].concat(),
            &F_HALF, // radius
            &[F0, F_QUARTER, F0].concat(),
            &[F0, F0, F0, F1].concat(),
            &[F_HALF, F_QUARTER, F1, F2].concat(),
            &[1, 0, 0, 0], // membership
            &[2, 0, 0, 0], // filter
            &[0],          // sensor
            &[1],          // events
            &[3, 0],       // material
            &[0],          // no joint
            &[0x42, 0, 0, 0, 0, 0, 0, 0],
            &[0], // parent
            &[7], // FootRight
            &[F0, F_HALF, F0, F0, F0, F0, F1].concat(),
            &[1], // a body
            &[F1, F0, F_HALF, F2].concat(),
            &[1], // kinematic
            &[0], // no locked axis
            &[0], // can_sleep
            &[2], // a cuboid
            &[F_HALF, F1, F2].concat(),
            &F0, // radius
            &[F_QUARTER, F0, F0].concat(),
            &[F_EIGHTH, F_QUARTER, F_HALF, W_BITS.to_le_bytes()].concat(),
            &[F1, F_HALF, F2, F_QUARTER].concat(),
            &[4, 0, 0, 0], // membership
            &[8, 0, 0, 0], // filter
            &[1],          // sensor
            &[0],          // events
            &[2, 1],       // material
            &[1],          // a joint
            &[2],          // revolute
            &[F0, F_QUARTER, F0].concat(),
            &[F0, F0, F_HALF].concat(),
            &[F1, F0, F0].concat(),
            &[1], // limited
            &[F_QUARTER, F1].concat(),
            &[1, 0, 0, 0], // one socket
            &[0x51, 0, 0, 0, 0, 0, 0, 0],
            &[1], // bone
            &[F0, F0, F0, F0, F0, F0, F1].concat(),
        ]
        .concat();
        assert_eq!(encode_skeleton(&skeleton), golden);
        // And anim's decoder reads those bytes as that skeleton, so the copy is the golden
        // value anim holds, not one this test made up.
        use lactose_asset_api::Loader;
        let loaded = lactose_anim::SkeletonLoader.load(&golden, &[]).unwrap();
        assert_eq!(loaded, skeleton);
    }

    /// Every role, shape, body type and joint kind through this encoder and anim's decoder:
    /// the golden skeleton holds only some of the variants, so a code this encoder writes
    /// for one variant and the decoder reads as another is caught here.
    #[test]
    fn every_variant_of_every_enum_reads_back_as_itself() {
        use lactose_asset_api::Loader;
        let roles = [
            BoneRole::Other,
            BoneRole::Hips,
            BoneRole::Spine,
            BoneRole::Head,
            BoneRole::HandLeft,
            BoneRole::HandRight,
            BoneRole::FootLeft,
            BoneRole::FootRight,
        ];
        let bodies = [RigidBody::dynamic(), RigidBody::kinematic(), RigidBody::fixed()];
        let shapes = [
            Shape::sphere(s(0.5)),
            Shape::capsule(s(0.5), s(0.25)),
            Shape::cuboid(v(0.5, 1.0, 2.0)),
        ];
        // Bone zero has nothing above it to join; bones one to four take the four kinds.
        let joints = [
            None,
            Some(JointKind::Fixed),
            Some(JointKind::Spherical),
            Some(JointKind::Revolute),
            Some(JointKind::Prismatic),
        ];
        let bones = roles
            .iter()
            .enumerate()
            .map(|(index, &role)| {
                let joint = joints.get(index).copied().flatten().map(|kind| BoneJoint {
                    kind,
                    anchor_parent: Vec3::ZERO,
                    anchor_bone: Vec3::ZERO,
                    axis: Vec3::Y,
                    limits: matches!(kind, JointKind::Revolute | JointKind::Prismatic)
                        .then(|| (s(-0.5), s(0.5))),
                });
                let collider = Collider::new(shapes[index % shapes.len()])
                    .with_groups(CollisionGroups::new(1 << index, 1 << 16));
                Bone {
                    key: BoneKey(0x100 + index as u64),
                    parent: index.checked_sub(1).map(|parent| u8::try_from(parent).unwrap()),
                    role,
                    rest: BonePose::IDENTITY,
                    body: Some(BoneBody { body: bodies[index % bodies.len()], collider, joint }),
                }
            })
            .collect();
        let skeleton = Skeleton { bones, sockets: Vec::new() };
        skeleton.validate().expect("a valid skeleton");
        let loaded = lactose_anim::SkeletonLoader.load(&encode_skeleton(&skeleton), &[]).unwrap();
        assert_eq!(loaded, skeleton);
    }
}
