//! The cooked byte form of the three asset kinds (`README.md`, "Assets", "The cooked byte
//! form"), checked against an encoder written here from that statement: each kind round
//! trips through its loader and through the store's erased loader, and each loader refuses
//! truncated bytes, trailing bytes, another format version or scalar width, a dependency
//! list other than the assets the bytes name, an unknown tag, and every `validate` failure.
//! RICOTTA's processors, which write these bytes, come with `tools/cook`; this encoder is
//! the test's alone.

#![forbid(unsafe_code)]

mod common;

use common::{MACHINE, SKELETON, bone_body, chain, clip, clip_ref, s, state, transition, v, yaw};
use lactose_anim::{
    AnimEventKey, Bone, BoneBody, BoneKey, BonePose, BoneRole, Clip, ClipEvent, ClipLoader,
    Condition, MAX_SIM_BONES, PARAM_SLOTS, Phase, RootSample, Skeleton, SkeletonLoader, Socket,
    StateKey, StateMachine, StateMachineLoader,
};
use lactose_asset_api::{
    Asset, AssetId, AssetRef, AssetVersion, CookedHeader, ErasedLoader, Loader,
};
use lactose_core::{ErrorKind, Result};
use lactose_math::{Canonical, Quat, Scalar, Vec3};
use lactose_physics_api::{BodyType, JointKind, ShapeKind};

// ---- The encoder: the README's statement of the byte form ----

/// Little-endian, fixed widths, in the order listed below the README's statement (not the
/// declaration order), no padding; a scalar is its canonical bits at the build's width; a
/// count is a `u32` before what it counts.
#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    /// The preamble: the format version and the scalar width.
    fn open(format: u32, width: u32) -> Self {
        let mut writer = Self::default();
        writer.u32(format);
        writer.u32(width);
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

    fn count(&mut self, count: usize) {
        self.u32(u32::try_from(count).expect("a small list"));
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

    fn asset<T>(&mut self, reference: AssetRef<T>) {
        self.u64(reference.id().to_bits());
    }

    fn index(&mut self, index: Option<u8>) {
        self.u8(index.unwrap_or(u8::MAX));
    }
}

const WIDTH: u32 = Scalar::BYTES as u32;

fn encode_clip(clip: &Clip) -> Vec<u8> {
    let mut w = Writer::open(Clip::FORMAT_VERSION, WIDTH);
    w.asset(clip.skeleton);
    w.u32(clip.duration_ticks);
    w.u32(clip.frames);
    w.bool(clip.looping);
    w.u8(clip.bones);
    w.count(clip.samples.len());
    clip.samples.iter().for_each(|sample| w.pose(sample));
    w.count(clip.root.len());
    for entry in &clip.root {
        w.vec3(entry.translation);
        w.scalar(entry.yaw);
    }
    w.count(clip.events.len());
    for event in &clip.events {
        w.u32(event.phase.0);
        w.u64(event.event.0);
    }
    w.0
}

fn encode_machine(machine: &StateMachine) -> Vec<u8> {
    let mut w = Writer::open(StateMachine::FORMAT_VERSION, WIDTH);
    w.u16(machine.initial);
    w.count(machine.states.len());
    for state in &machine.states {
        w.u64(state.key.0);
        w.asset(state.clip);
        w.scalar(state.speed);
        w.index(state.speed_param);
    }
    w.count(machine.transitions.len());
    for transition in &machine.transitions {
        w.u16(transition.from.unwrap_or(u16::MAX));
        w.u16(transition.to);
        w.u16(transition.blend_ticks);
        w.bool(transition.keep_phase);
        w.u32(transition.entry_phase.0);
        w.count(transition.conditions.len());
        for condition in &transition.conditions {
            match *condition {
                Condition::Finished => w.u8(0),
                Condition::PhaseAtLeast(phase) => {
                    w.u8(1);
                    w.u32(phase.0);
                }
                Condition::TicksAtLeast(ticks) => {
                    w.u8(2);
                    w.u32(ticks);
                }
                Condition::Above { param, value } => {
                    w.u8(3);
                    w.u8(param);
                    w.scalar(value);
                }
                Condition::Below { param, value } => {
                    w.u8(4);
                    w.u8(param);
                    w.scalar(value);
                }
                Condition::Flag { bit, set } => {
                    w.u8(5);
                    w.u8(bit);
                    w.bool(set);
                }
                Condition::Trigger(bit) => {
                    w.u8(6);
                    w.u8(bit);
                }
            }
        }
    }
    w.0
}

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

fn encode_body(w: &mut Writer, body: &BoneBody) {
    let rigid = &body.body;
    w.scalar(rigid.mass);
    w.scalar(rigid.linear_damping);
    w.scalar(rigid.angular_damping);
    w.scalar(rigid.gravity_scale);
    w.u8(match rigid.body_type {
        BodyType::Dynamic => 0,
        BodyType::Kinematic => 1,
        BodyType::Fixed => 2,
    });
    w.u8(rigid.locked_axes);
    w.bool(rigid.can_sleep);
    let collider = &body.collider;
    w.u8(match collider.shape.kind {
        ShapeKind::Sphere => 0,
        ShapeKind::Capsule => 1,
        ShapeKind::Cuboid => 2,
    });
    w.vec3(collider.shape.half_extents);
    w.scalar(collider.shape.radius);
    w.vec3(collider.offset);
    w.quat(collider.rotation);
    w.scalar(collider.friction);
    w.scalar(collider.restitution);
    w.scalar(collider.density);
    w.scalar(collider.impact_threshold);
    w.u32(collider.groups.membership);
    w.u32(collider.groups.filter);
    w.bool(collider.sensor);
    w.bool(collider.events);
    w.u16(collider.material);
    w.bool(body.joint.is_some());
    if let Some(joint) = &body.joint {
        w.u8(match joint.kind {
            JointKind::Fixed => 0,
            JointKind::Spherical => 1,
            JointKind::Revolute => 2,
            JointKind::Prismatic => 3,
        });
        w.vec3(joint.anchor_parent);
        w.vec3(joint.anchor_bone);
        w.vec3(joint.axis);
        w.bool(joint.limits.is_some());
        if let Some((min, max)) = joint.limits {
            w.scalar(min);
            w.scalar(max);
        }
    }
}

fn encode_skeleton(skeleton: &Skeleton) -> Vec<u8> {
    let mut w = Writer::open(Skeleton::FORMAT_VERSION, WIDTH);
    w.count(skeleton.bones.len());
    for bone in &skeleton.bones {
        w.u64(bone.key.0);
        w.index(bone.parent);
        w.u8(role(bone.role));
        w.pose(&bone.rest);
        w.bool(bone.body.is_some());
        if let Some(body) = &bone.body {
            encode_body(&mut w, body);
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

// ---- The assets ----

const WALK: AssetRef<Clip> = clip_ref("format/walk");
const JUMP: AssetRef<Clip> = clip_ref("format/jump");

/// A looping clip with a root track and events on two bones.
fn sample_clip() -> Clip {
    let mut clip = common::with_root(clip(2, 3, 24, true, 0.3), 0.2, 0.01);
    clip.events = vec![
        ClipEvent { phase: Phase::ZERO, event: AnimEventKey(7) },
        ClipEvent { phase: Phase::HALF, event: AnimEventKey(u64::MAX) },
        ClipEvent { phase: Phase::HALF, event: AnimEventKey(1) },
    ];
    clip
}

/// Every condition, an any-state transition, a kept and a set entry phase.
fn sample_machine() -> StateMachine {
    let mut walk = state("walk", WALK, 1.25);
    walk.speed_param = Some(3);
    let mut run = transition(
        Some(0),
        1,
        &[
            Condition::Finished,
            Condition::PhaseAtLeast(Phase(123_456_789)),
            Condition::TicksAtLeast(17),
            Condition::Above { param: 3, value: s(0.5) },
            Condition::Below { param: 7, value: s(-2.0) },
            Condition::Flag { bit: 31, set: false },
        ],
    );
    run.blend_ticks = 9;
    run.keep_phase = true;
    let mut jump =
        transition(None, 1, &[Condition::Trigger(4), Condition::Flag { bit: 2, set: true }]);
    jump.entry_phase = Phase::from_fraction(1, 4);
    StateMachine {
        states: vec![walk, state("jump", JUMP, 0.0)],
        transitions: vec![run, jump, transition(Some(1), 0, &[])],
        initial: 1,
    }
}

/// Four bones: a body with every field away from its default and a limited revolute joint
/// on bone 2, a plain body on bone 1, none on 0 and 3; two sockets.
fn sample_skeleton() -> Skeleton {
    let mut skeleton = chain(4, |index| index == 1 || index == 2);
    let body = skeleton.bones[2].body.as_mut().expect("bone 2 has a body");
    body.body.mass = s(3.5);
    body.body.linear_damping = s(0.25);
    body.body.angular_damping = s(0.75);
    body.body.gravity_scale = s(-1.0);
    body.body.body_type = BodyType::Kinematic;
    body.body.locked_axes = 0b101;
    body.body.can_sleep = false;
    body.collider.shape.kind = ShapeKind::Capsule;
    body.collider.shape.half_extents = v(0.1, 0.2, 0.3);
    body.collider.shape.radius = s(0.15);
    body.collider.offset = v(0.0, 0.5, 0.0);
    body.collider.rotation = yaw(0.3);
    body.collider.friction = s(0.9);
    body.collider.restitution = s(0.1);
    body.collider.density = s(2.0);
    body.collider.impact_threshold = s(4.0);
    body.collider.sensor = true;
    body.collider.events = true;
    body.collider.material = 0xBEEF;
    let joint = body.joint.as_mut().expect("bone 2 is jointed");
    joint.kind = JointKind::Revolute;
    joint.anchor_bone = v(0.0, -0.1, 0.0);
    joint.axis = Vec3::X;
    joint.limits = Some((s(-0.5), s(1.25)));
    skeleton.sockets.push(Socket {
        key: BoneKey(901),
        bone: 0,
        offset: BonePose::new(v(0.1, 0.0, 0.0), yaw(-0.2)),
    });
    skeleton
}

// ---- Loading ----

fn load_clip(bytes: &[u8], dependencies: &[AssetId]) -> Result<Clip> {
    ClipLoader.load(bytes, dependencies)
}

fn load_machine(bytes: &[u8], dependencies: &[AssetId]) -> Result<StateMachine> {
    StateMachineLoader.load(bytes, dependencies)
}

fn load_skeleton(bytes: &[u8], dependencies: &[AssetId]) -> Result<Skeleton> {
    SkeletonLoader.load(bytes, dependencies)
}

fn clip_deps() -> Vec<AssetId> {
    vec![SKELETON.id()]
}

fn machine_deps() -> Vec<AssetId> {
    let mut deps = vec![WALK.id(), JUMP.id()];
    deps.sort_unstable();
    deps
}

/// The kind and message of a refusal.
fn refusal<T: core::fmt::Debug>(result: Result<T>) -> (ErrorKind, String) {
    let error = result.expect_err("the loader refuses");
    (error.kind(), error.message().to_owned())
}

#[track_caller]
fn assert_invalid<T: core::fmt::Debug>(result: Result<T>, says: &str) {
    let (kind, message) = refusal(result);
    assert_eq!(kind, ErrorKind::InvalidData, "{message}");
    assert!(message.contains(says), "{message:?} does not say {says:?}");
}

// ---- Round trips ----

#[test]
fn each_kind_round_trips_through_its_loader() {
    let clip = sample_clip();
    clip.validate().unwrap();
    let bytes = encode_clip(&clip);
    assert_eq!(load_clip(&bytes, &clip_deps()).unwrap(), clip);

    let machine = sample_machine();
    machine.validate().unwrap();
    let bytes = encode_machine(&machine);
    assert_eq!(load_machine(&bytes, &machine_deps()).unwrap(), machine);

    let skeleton = sample_skeleton();
    skeleton.validate().unwrap();
    let bytes = encode_skeleton(&skeleton);
    assert_eq!(load_skeleton(&bytes, &[]).unwrap(), skeleton);
}

#[test]
fn the_byte_counts_are_the_stated_widths() {
    let s = Scalar::BYTES;
    let pose = 7 * s;
    let clip = sample_clip();
    let (samples, root, events) = (clip.samples.len(), clip.root.len(), clip.events.len());
    let expected =
        8 + (8 + 4 + 4 + 1 + 1) + 4 + samples * pose + 4 + root * 4 * s + 4 + events * 12;
    assert_eq!(encode_clip(&clip).len(), expected);
    // The preamble: format 1, then the scalar width, little-endian.
    assert_eq!(encode_clip(&clip)[..8], [1, 0, 0, 0, Scalar::BYTES as u8, 0, 0, 0]);

    let machine = sample_machine();
    let conditions: usize = [1 + 4, 1 + 4, 1 + 1 + s, 1 + 1 + s, 1 + 1 + 1].iter().sum::<usize>()
        + 1
        + (1 + 1)
        + (1 + 1 + 1);
    let expected = 8 + 2 + 4 + 2 * (8 + 8 + s + 1) + 4 + 3 * (2 + 2 + 2 + 1 + 4 + 4) + conditions;
    assert_eq!(encode_machine(&machine).len(), expected);

    let skeleton = sample_skeleton();
    let body = 4 * s + 3 + (1 + 3 * s + s + 3 * s + 4 * s + 4 * s + 4 + 4 + 1 + 1 + 2) + 1;
    let joint = 1 + 9 * s + 1;
    let expected =
        8 + 4 + 4 * (8 + 1 + 1 + pose + 1) + 2 * body + joint + 2 * s + 4 + 2 * (8 + 1 + pose);
    assert_eq!(encode_skeleton(&skeleton).len(), expected);
}

#[test]
fn each_kind_round_trips_through_the_erased_loader() {
    fn through<L: Loader>(loader: L, bytes: &[u8], deps: &[AssetId]) -> L::Asset
    where
        L::Asset: Clone,
    {
        let header = CookedHeader::new(
            <L::Asset as Asset>::KIND,
            <L::Asset as Asset>::FORMAT_VERSION,
            deps.iter().copied(),
        );
        let version: AssetVersion = header.version_of(bytes);
        let id = AssetId::from_name("format/erased");
        let handle = ErasedLoader::new(loader).load(id, version, &header, bytes).unwrap();
        assert_eq!((handle.id(), handle.version()), (id, version));
        (*handle.downcast::<L::Asset>().expect("the loader's kind")).clone()
    }
    let clip = sample_clip();
    assert_eq!(through(ClipLoader, &encode_clip(&clip), &clip_deps()), clip);
    let machine = sample_machine();
    assert_eq!(through(StateMachineLoader, &encode_machine(&machine), &machine_deps()), machine);
    let skeleton = sample_skeleton();
    assert_eq!(through(SkeletonLoader, &encode_skeleton(&skeleton), &[]), skeleton);
}

// ---- Refusals of the bytes ----

/// One kind's cooked bytes and its loader, which gives the kind and message of a refusal.
type Case = (Vec<u8>, fn(&[u8]) -> Option<(ErrorKind, String)>);

#[test]
fn every_truncation_and_any_trailing_byte_is_refused() {
    let cases: [Case; 3] = [
        (encode_clip(&sample_clip()), |b| {
            load_clip(b, &clip_deps()).err().map(|e| (e.kind(), e.message().to_owned()))
        }),
        (encode_machine(&sample_machine()), |b| {
            load_machine(b, &machine_deps()).err().map(|e| (e.kind(), e.message().to_owned()))
        }),
        (encode_skeleton(&sample_skeleton()), |b| {
            load_skeleton(b, &[]).err().map(|e| (e.kind(), e.message().to_owned()))
        }),
    ];
    for (bytes, load) in cases {
        assert!(load(&bytes).is_none());
        for end in 0..bytes.len() {
            let (kind, message) = load(&bytes[..end]).expect("a truncation is refused");
            assert_eq!(kind, ErrorKind::InvalidData, "{end} of {} bytes: {message}", bytes.len());
            assert!(message.contains("end inside"), "{end}: {message}");
        }
        for extra in [&[0u8][..], &[1, 2, 3], &[0; 64]] {
            let mut longer = bytes.clone();
            longer.extend_from_slice(extra);
            let (kind, message) = load(&longer).expect("trailing bytes are refused");
            assert_eq!(kind, ErrorKind::InvalidData);
            assert!(message.contains("run on past the asset"), "{message}");
        }
    }
}

#[test]
fn another_format_or_scalar_width_is_a_version_mismatch() {
    let clip = encode_clip(&sample_clip());
    let machine = encode_machine(&sample_machine());
    let skeleton = encode_skeleton(&sample_skeleton());
    for (offset, value) in [(0, 2u32), (0, 0), (4, 4), (4, 16)] {
        let patch = |mut bytes: Vec<u8>| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            bytes
        };
        for (kind, _) in [
            refusal(load_clip(&patch(clip.clone()), &clip_deps())),
            refusal(load_machine(&patch(machine.clone()), &machine_deps())),
            refusal(load_skeleton(&patch(skeleton.clone()), &[])),
        ] {
            assert_eq!(kind, ErrorKind::VersionMismatch, "{value} at {offset}");
        }
    }
}

#[test]
fn a_dependency_list_other_than_the_named_assets_is_refused() {
    let clip = encode_clip(&sample_clip());
    let other = AssetId::from_name("format/other");
    for deps in [vec![], vec![other], vec![SKELETON.id(), other]] {
        let mut deps = deps;
        deps.sort_unstable();
        assert_invalid(load_clip(&clip, &deps), "dependencies");
    }
    let machine = encode_machine(&sample_machine());
    for deps in
        [vec![WALK.id()], vec![JUMP.id()], vec![], [machine_deps(), vec![MACHINE.id()]].concat()]
    {
        let mut deps = deps;
        deps.sort_unstable();
        assert_invalid(load_machine(&machine, &deps), "dependencies");
    }
    assert_invalid(load_skeleton(&encode_skeleton(&sample_skeleton()), &[other]), "dependencies");
}

/// Replaces the bytes at `at` of `bytes` with `with`.
fn patched(bytes: &[u8], at: usize, with: &[u8]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    bytes[at..at + with.len()].copy_from_slice(with);
    bytes
}

#[test]
fn an_unknown_tag_or_a_bool_other_than_zero_or_one_is_refused() {
    let s = Scalar::BYTES;
    // The clip's looping byte: preamble, skeleton, duration, frames.
    let clip = encode_clip(&sample_clip());
    assert_invalid(load_clip(&patched(&clip, 8 + 16, &[2]), &clip_deps()), "not 0 or 1");

    // The first condition's tag: preamble, initial, two states, the transition count and the
    // first transition's fixed fields and condition count.
    let machine = encode_machine(&sample_machine());
    let first_tag = 8 + 2 + 4 + 2 * (8 + 8 + s + 1) + 4 + (2 + 2 + 2 + 1 + 4) + 4;
    assert_eq!(machine[first_tag], 0, "Finished");
    assert_invalid(
        load_machine(&patched(&machine, first_tag, &[7]), &machine_deps()),
        "unknown tag 7",
    );

    // Bone 0's role: preamble, the bone count, key, parent.
    let skeleton = encode_skeleton(&sample_skeleton());
    let role_at = 8 + 4 + 8 + 1;
    assert_eq!(skeleton[role_at], 1, "Hips");
    assert_invalid(load_skeleton(&patched(&skeleton, role_at, &[8]), &[]), "unknown role 8");
    // Bone 1's body: after bone 0 (no body) and bone 1's fixed fields and presence byte.
    let bone = 8 + 1 + 1 + 7 * s + 1;
    let body_at = 8 + 4 + bone + bone;
    assert_eq!(skeleton[body_at - 1], 1, "bone 1 has a body");
    let body_type = body_at + 4 * s;
    assert_invalid(load_skeleton(&patched(&skeleton, body_type, &[3]), &[]), "unknown type 3");
    let shape = body_type + 3;
    assert_invalid(load_skeleton(&patched(&skeleton, shape, &[3]), &[]), "unknown shape 3");
    let joint = shape + 1 + 3 * s + s + 3 * s + 4 * s + 4 * s + 4 + 4 + 1 + 1 + 2;
    assert_eq!(skeleton[joint], 0, "bone 1's body is the ragdoll's root: no joint");
    assert_invalid(load_skeleton(&patched(&skeleton, joint, &[2]), &[]), "not 0 or 1");
    // Bone 2's joint kind, past bone 1's body and bone 2's fixed fields and body.
    let body_bytes = joint + 1 - body_at;
    let kind = body_at + body_bytes + bone + body_bytes;
    assert_eq!(skeleton[kind - 1], 1, "bone 2 is jointed");
    assert_eq!(skeleton[kind], 2, "Revolute");
    assert_invalid(load_skeleton(&patched(&skeleton, kind, &[4]), &[]), "unknown kind 4");
}

#[test]
fn a_count_the_bytes_cannot_hold_is_refused_before_allocating() {
    // The clip's sample count, after the preamble and the fixed fields.
    let clip = encode_clip(&sample_clip());
    let at = 8 + 8 + 4 + 4 + 1 + 1;
    assert_eq!(clip[at..at + 4], 6u32.to_le_bytes());
    for count in [10, u32::MAX] {
        let bytes = patched(&clip, at, &count.to_le_bytes());
        assert_invalid(load_clip(&bytes, &clip_deps()), "end inside the clip's samples");
    }
}

// ---- Every validate failure, through the loader ----

#[track_caller]
fn refused_clip(change: impl FnOnce(&mut Clip), says: &str) {
    let mut clip = sample_clip();
    change(&mut clip);
    let deps = [clip.skeleton.id()];
    assert_invalid(load_clip(&encode_clip(&clip), &deps), says);
}

#[test]
fn every_clip_validation_failure_is_refused() {
    refused_clip(|c| c.duration_ticks = 0, "at least one tick");
    refused_clip(
        |c| {
            c.looping = false;
            c.frames = 1;
            c.samples.truncate(2);
            c.root.clear();
        },
        "two frames",
    );
    refused_clip(|c| c.samples.truncate(5), "samples, not 5");
    refused_clip(|c| c.root.truncate(3), "root track holds 4 entries or none, not 3");
    refused_clip(
        |c| c.samples[4].rotation = Quat::from_array([s(0.0); 4]),
        "sample 4: the rotation",
    );
    refused_clip(
        |c| c.samples[1].translation = v(f64::NAN, 0.0, 0.0),
        "sample 1: a scalar is not finite",
    );
    refused_clip(
        |c| c.root[2] = RootSample { translation: Vec3::ZERO, yaw: s(f64::INFINITY) },
        "root entry 2",
    );
    refused_clip(|c| c.root[0].yaw = s(0.3), "root entry 0: the track does not start at");
    refused_clip(|c| c.root[0].translation.x = s(0.5), "root entry 0: the track does not");
    refused_clip(|c| c.events[0].phase = Phase::END, "event 1: the events are not ascending");
}

#[track_caller]
fn refused_machine(change: impl FnOnce(&mut StateMachine), says: &str) {
    let mut machine = sample_machine();
    change(&mut machine);
    let mut deps: Vec<AssetId> = machine.states.iter().map(|state| state.clip.id()).collect();
    deps.sort_unstable();
    deps.dedup();
    assert_invalid(load_machine(&encode_machine(&machine), &deps), says);
}

#[test]
fn every_state_machine_validation_failure_is_refused() {
    refused_machine(
        |m| {
            m.states.clear();
            m.transitions.clear();
        },
        "at least one state",
    );
    refused_machine(|m| m.initial = 2, "the initial state 2 is past the states");
    refused_machine(|m| m.states[1].speed = s(-0.5), "state 1: the speed is negative");
    refused_machine(
        |m| m.states[0].speed = s(f64::NAN),
        "state 0: the speed is negative or not finite",
    );
    refused_machine(
        |m| m.states[0].speed_param = Some(PARAM_SLOTS as u8),
        "state 0: the speed parameter is past the slots",
    );
    refused_machine(
        |m| m.states[1].key = StateKey::from_name("walk"),
        "state 1: another state has its key",
    );
    refused_machine(|m| m.transitions[2].from = Some(2), "transition 2: the state it leaves");
    refused_machine(|m| m.transitions[1].to = 5, "transition 1: the state it enters");
    refused_machine(
        |m| m.transitions[0].conditions[3] = Condition::Above { param: 8, value: s(1.0) },
        "transition 0: a condition's parameter",
    );
    refused_machine(
        |m| m.transitions[0].conditions[4] = Condition::Below { param: 200, value: s(1.0) },
        "transition 0: a condition's parameter",
    );
    refused_machine(
        |m| m.transitions[1].conditions[0] = Condition::Trigger(32),
        "transition 1: a condition's bit",
    );
    refused_machine(
        |m| m.transitions[0].conditions[5] = Condition::Flag { bit: 40, set: true },
        "transition 0: a condition's bit",
    );
}

#[track_caller]
fn refused_skeleton(change: impl FnOnce(&mut Skeleton), says: &str) {
    let mut skeleton = sample_skeleton();
    change(&mut skeleton);
    assert_invalid(load_skeleton(&encode_skeleton(&skeleton), &[]), says);
}

#[test]
fn every_skeleton_validation_failure_is_refused() {
    refused_skeleton(
        |k| {
            k.bones.clear();
            k.sockets.clear();
        },
        "at least one bone",
    );
    refused_skeleton(
        |k| {
            let template = k.bones[3];
            k.bones.extend((4..=MAX_SIM_BONES).map(|index| Bone {
                key: BoneKey(100 + index as u64),
                parent: Some(index as u8 - 1),
                ..template
            }));
        },
        "at most 16, not 17",
    );
    refused_skeleton(|k| k.bones[0].parent = Some(0), "bone 0: the root bone has a parent");
    refused_skeleton(|k| k.bones[2].parent = None, "bone 2: only bone zero has no parent");
    refused_skeleton(|k| k.bones[2].parent = Some(3), "bone 2: the parent does not precede");
    refused_skeleton(|k| k.bones[3].key = BoneKey(100), "bone 3: another bone has its key");
    refused_skeleton(|k| k.bones[3].role = BoneRole::Hips, "bone 3: another bone has its role");
    refused_skeleton(
        |k| k.bones[1].rest.rotation = Quat::from_array([s(1.0); 4]),
        "bone 1: the rest pose: the rotation",
    );
    refused_skeleton(
        |k| k.bones[1].rest.translation = v(0.0, f64::INFINITY, 0.0),
        "bone 1: the rest pose: a scalar",
    );
    refused_skeleton(
        |k| k.bones[1].body.as_mut().unwrap().collider.friction = s(f64::NAN),
        "bone 1: a scalar of the body is not finite",
    );
    refused_skeleton(
        |k| {
            let joint = k.bones[2].body.as_mut().unwrap().joint.as_mut().unwrap();
            joint.limits = Some((s(0.0), s(f64::NEG_INFINITY)));
        },
        "bone 2: a scalar of the body",
    );
    refused_skeleton(
        |k| k.bones[1].body = Some(bone_body(true)),
        "bone 1: the joint reaches no ancestor with a body",
    );
    refused_skeleton(
        |k| {
            let groups = &mut k.bones[2].body.as_mut().unwrap().collider.groups;
            groups.filter |= common::BONE_GROUP;
        },
        "memberships 0x2 meet their filters",
    );
    refused_skeleton(
        |k| {
            // (membership 1, filter 2) and (membership 2, filter 1): each admits the other.
            let one = &mut k.bones[1].body.as_mut().unwrap().collider.groups;
            (one.membership, one.filter) = (1, 2);
            let two = &mut k.bones[2].body.as_mut().unwrap().collider.groups;
            (two.membership, two.filter) = (2, 1);
        },
        "meet their filters",
    );
    refused_skeleton(|k| k.sockets[1].bone = 4, "socket 1: the bone is past the bones");
    refused_skeleton(
        |k| k.sockets[0].offset.rotation = Quat::from_array([s(0.0), s(0.0), s(0.0), s(2.0)]),
        "socket 0: the offset: the rotation",
    );
}

// ---- Golden bytes ----

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

fn concat(pieces: &[&[u8]]) -> Vec<u8> {
    pieces.concat()
}

/// One small asset of each kind against bytes written by hand from the README's per-kind
/// lists, independent of the encoder above: each decodes to the expected value and the
/// encoder writes exactly those bytes. Every two neighbouring fields of one width hold
/// different values in at least one asset, and every condition tag, a transition from any
/// state, a sphere and a cuboid, two body types and a limited revolute joint appear, so a
/// swap or a width the encoder and the decoder share is caught here.
#[test]
fn one_asset_of_each_kind_matches_its_golden_bytes() {
    use lactose_anim::{BoneJoint, State, Transition};
    use lactose_physics_api::{Collider, CollisionGroups, RigidBody, Shape, locked_axes};
    assert_eq!(Scalar::BYTES, 8, "the golden bytes are at eight-byte scalars");
    let skeleton_id = AssetId::from_bits(0x0102_0304_0506_0708);
    let clip_id = AssetId::from_bits(0x31);

    // A one-shot clip of three ticks, two frames and one bone, its root entry zero the
    // identity, and one event. Every two neighbours of one width differ throughout, so a
    // swap of them shared by the encoder and the decoder changes the bytes.
    let clip = Clip {
        skeleton: AssetRef::new(skeleton_id),
        duration_ticks: 3,
        frames: 2,
        looping: false,
        bones: 1,
        samples: vec![
            BonePose { translation: v(1.0, 0.0, 0.0), rotation: Quat::IDENTITY },
            BonePose { translation: v(2.0, 0.0, 0.0), rotation: Quat::IDENTITY },
        ],
        root: vec![
            RootSample { translation: Vec3::ZERO, yaw: Scalar::ZERO },
            RootSample { translation: v(0.0, 0.0, 1.0), yaw: s(0.25) },
        ],
        events: vec![ClipEvent { phase: Phase(0x4000_0000), event: AnimEventKey(0x11) }],
    };
    let golden = concat(&[
        &PREAMBLE,
        &[8, 7, 6, 5, 4, 3, 2, 1], // skeleton
        &[3, 0, 0, 0],             // duration_ticks
        &[2, 0, 0, 0],             // frames
        &[0],                      // looping
        &[1],                      // bones
        &[2, 0, 0, 0],             // two samples
        &[F1, F0, F0, F0, F0, F0, F1].concat(),
        &[F2, F0, F0, F0, F0, F0, F1].concat(),
        &[2, 0, 0, 0], // two root entries
        &[F0, F0, F0, F0].concat(),
        &[F0, F0, F1, F_QUARTER].concat(),
        &[1, 0, 0, 0], // one event
        &[0, 0, 0, 0x40],
        &[0x11, 0, 0, 0, 0, 0, 0, 0],
    ]);
    assert_eq!(load_clip(&golden, &[skeleton_id]).unwrap(), clip);
    assert_eq!(encode_clip(&clip), golden);

    // Two states, a transition between them and one from any state, one condition of each
    // tag: `initial` first, a transition's conditions last.
    let walk_id = AssetId::from_bits(0x32);
    let machine = StateMachine {
        states: vec![
            State {
                key: StateKey(0x21),
                clip: AssetRef::new(clip_id),
                speed: s(1.0),
                speed_param: None,
            },
            State {
                key: StateKey(0x22),
                clip: AssetRef::new(walk_id),
                speed: s(0.5),
                speed_param: Some(3),
            },
        ],
        transitions: vec![
            Transition {
                from: Some(0),
                to: 1,
                conditions: vec![
                    Condition::Finished,
                    Condition::PhaseAtLeast(Phase(0x8000_0000)),
                    Condition::TicksAtLeast(9),
                ],
                blend_ticks: 4,
                keep_phase: true,
                entry_phase: Phase::ZERO,
            },
            Transition {
                from: None,
                to: 0,
                conditions: vec![
                    Condition::Above { param: 1, value: s(0.5) },
                    Condition::Below { param: 2, value: s(0.25) },
                    Condition::Flag { bit: 3, set: true },
                    Condition::Trigger(5),
                ],
                blend_ticks: 6,
                keep_phase: false,
                entry_phase: Phase(0x2000_0000),
            },
        ],
        initial: 1,
    };
    let golden = concat(&[
        &PREAMBLE,
        &[1, 0],       // initial
        &[2, 0, 0, 0], // two states
        &[0x21, 0, 0, 0, 0, 0, 0, 0],
        &[0x31, 0, 0, 0, 0, 0, 0, 0],
        &F1,
        &[0xFF], // no speed parameter
        &[0x22, 0, 0, 0, 0, 0, 0, 0],
        &[0x32, 0, 0, 0, 0, 0, 0, 0],
        &F_HALF,
        &[3],                // speed parameter 3
        &[2, 0, 0, 0],       // two transitions
        &[0, 0],             // from
        &[1, 0],             // to
        &[4, 0],             // blend_ticks
        &[1],                // keep_phase
        &[0, 0, 0, 0],       // entry_phase
        &[3, 0, 0, 0],       // three conditions
        &[0],                // Finished
        &[1, 0, 0, 0, 0x80], // PhaseAtLeast
        &[2, 9, 0, 0, 0],    // TicksAtLeast(9)
        &[0xFF, 0xFF],       // from any state
        &[0, 0],             // to
        &[6, 0],             // blend_ticks
        &[0],                // keep_phase
        &[0, 0, 0, 0x20],    // entry_phase
        &[4, 0, 0, 0],       // four conditions
        &[3, 1],             // Above, parameter 1
        &F_HALF,
        &[4, 2], // Below, parameter 2
        &F_QUARTER,
        &[5, 3, 1], // Flag, bit 3, set
        &[6, 5],    // Trigger(5)
    ]);
    assert_eq!(load_machine(&golden, &[clip_id, walk_id]).unwrap(), machine);
    assert_eq!(encode_machine(&machine), golden);

    // Two bones with bodies, the second a kinematic cuboid on a limited revolute joint to
    // the first, and one socket: a shape's kind before its extents.
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
                body: Some(BoneBody { body: foot, collider: foot_collider, joint: Some(hinge) }),
            },
        ],
        sockets: vec![Socket { key: BoneKey(0x51), bone: 1, offset: BonePose::IDENTITY }],
    };
    let golden = concat(&[
        &PREAMBLE,
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
    ]);
    assert_eq!(load_skeleton(&golden, &[]).unwrap(), skeleton);
    assert_eq!(encode_skeleton(&skeleton), golden);
}

/// The largest machine the format holds, 65,534 states, is checked by sorting rather than
/// pairwise; the result is the same first failure, in index order with the other checks.
#[test]
fn a_machine_of_65534_states_names_its_first_repeated_key() {
    const STATES: usize = u16::MAX as usize - 1;
    let mut machine = StateMachine {
        states: (0..STATES)
            .map(|index| {
                let mut state = state("any", WALK, 1.0);
                state.key = StateKey(u64::MAX - index as u64 * 7_919);
                state
            })
            .collect(),
        transitions: Vec::new(),
        initial: 0,
    };
    machine.validate().unwrap();
    let bytes = encode_machine(&machine);
    assert_eq!(load_machine(&bytes, &[WALK.id()]).unwrap(), machine);
    // Two repeats: 65,533 repeats 40,000's key and 50,000 repeats 3's; 50,000 is first.
    machine.states[40_000].key = machine.states[STATES - 1].key;
    machine.states[50_000].key = machine.states[3].key;
    let error = machine.validate().unwrap_err();
    assert_eq!(error.message(), "state 50000: another state has its key");
    let error = load_machine(&encode_machine(&machine), &[WALK.id()]).unwrap_err();
    assert_eq!(error.message(), "state 50000: another state has its key");
    // An earlier state's other failure still comes first.
    machine.states[45_000].speed = s(-1.0);
    let error = machine.validate().unwrap_err();
    assert_eq!(error.message(), "state 45000: the speed is negative or not finite");
    machine.states[45_000].speed = s(1.0);
    machine.states[50_000].key = StateKey(1);
    let error = machine.validate().unwrap_err();
    assert_eq!(error.message(), format!("state {}: another state has its key", STATES - 1));
}
