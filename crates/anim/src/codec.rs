//! The cooked byte form of the three asset kinds (`README.md`, "Assets"): little-endian,
//! every field at a fixed width in the order listed below the README's statement of the
//! form, which is not always the declaration order (`StateMachine::initial`, a
//! `Transition`'s conditions and a `Shape`'s kind are not), with no alignment and no
//! padding, every scalar its canonical bits at the width of the build's `Scalar`, every
//! count a `u32` before the elements it counts. Decoding reads integers and canonical bits and
//! nothing else: no float is parsed and no layout of the host is observed, so a decode is
//! bit-identical on every host. A loader decodes, checks the declared dependencies against
//! the assets the bytes name, then validates.

use lactose_asset_api::{Asset, AssetId, AssetRef};
use lactose_core::{Error, ErrorKind, Result};
use lactose_math::{Canonical, Quat, Scalar, Vec3};
use lactose_physics_api::{
    BodyType, Collider, CollisionGroups, JointKind, RigidBody, Shape, ShapeKind,
};

use crate::assets::{
    Bone, BoneBody, BoneJoint, BoneRole, Clip, ClipEvent, Condition, RootSample, Skeleton, Socket,
    State, StateMachine, Transition, invalid,
};
use crate::components::BonePose;
use crate::keys::{AnimEventKey, BoneKey, StateKey};
use crate::phase::Phase;

/// The byte that stands for `None` in an optional bone or parameter index; both are below
/// it in every asset `validate` accepts.
const NO_INDEX: u8 = u8::MAX;

/// The word that stands for `None` in an optional state index; a machine has fewer than
/// `u16::MAX` states.
const NO_STATE: u16 = u16::MAX;

/// The tags of the [`Condition`] variants, in declaration order.
const FINISHED: u8 = 0;
const PHASE_AT_LEAST: u8 = 1;
const TICKS_AT_LEAST: u8 = 2;
const ABOVE: u8 = 3;
const BELOW: u8 = 4;
const FLAG: u8 = 5;
const TRIGGER: u8 = 6;

const S: usize = Scalar::BYTES;
/// The smallest encoding of each counted element, so a count the remaining bytes cannot
/// hold is refused before anything is allocated for it.
const POSE_BYTES: usize = 7 * S;
const ROOT_BYTES: usize = 4 * S;
const EVENT_BYTES: usize = 4 + 8;
const STATE_BYTES: usize = 8 + 8 + S + 1;
const TRANSITION_BYTES: usize = 2 + 2 + 2 + 1 + 4 + 4;
const CONDITION_BYTES: usize = 1;
const BONE_BYTES: usize = 8 + 1 + 1 + POSE_BYTES + 1;
const SOCKET_BYTES: usize = 8 + 1 + POSE_BYTES;

/// A cursor over cooked bytes; every read refuses bytes that end inside the field.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader past the preamble of an asset of kind `T`: the format version and the
    /// scalar width, each a `u32`, which must be `T::FORMAT_VERSION` and `Scalar::BYTES`.
    fn open<T: Asset>(bytes: &'a [u8]) -> Result<Self> {
        let mut reader = Self { bytes };
        let format = reader.u32("the format version")?;
        if format != T::FORMAT_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!("cooked bytes of format {format}, not {}", T::FORMAT_VERSION),
            ));
        }
        let width = reader.u32("the scalar width")?;
        if width as usize != S {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!("cooked bytes of {width}-byte scalars, not {S}"),
            ));
        }
        Ok(reader)
    }

    /// Refuses bytes left over after the asset.
    fn finish(self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(invalid(format!("{} bytes run on past the asset", self.bytes.len())))
        }
    }

    fn take<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N]> {
        let Some((head, rest)) = self.bytes.split_first_chunk::<N>() else {
            return Err(invalid(format!("the cooked bytes end inside {what}")));
        };
        self.bytes = rest;
        Ok(*head)
    }

    fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(u8::from_le_bytes(self.take(what)?))
    }

    fn u16(&mut self, what: &'static str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(what)?))
    }

    fn u32(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(what)?))
    }

    fn u64(&mut self, what: &'static str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(what)?))
    }

    fn bool(&mut self, what: &'static str) -> Result<bool> {
        match self.u8(what)? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(invalid(format!("{what} is the byte {other}, not 0 or 1"))),
        }
    }

    fn scalar(&mut self, what: &'static str) -> Result<Scalar> {
        Ok(Scalar::from_canonical_bytes(self.take(what)?))
    }

    fn vec3(&mut self, what: &'static str) -> Result<Vec3> {
        Ok(Vec3::from_canonical_bytes(self.take(what)?))
    }

    fn quat(&mut self, what: &'static str) -> Result<Quat> {
        Ok(Quat::from_canonical_bytes(self.take(what)?))
    }

    fn pose(&mut self, what: &'static str) -> Result<BonePose> {
        Ok(BonePose::new(self.vec3(what)?, self.quat(what)?))
    }

    fn asset<T>(&mut self, what: &'static str) -> Result<AssetRef<T>> {
        Ok(AssetRef::new(AssetId::from_bits(self.u64(what)?)))
    }

    /// A count of elements of at least `element` bytes each, refused when the remaining
    /// bytes cannot hold that many.
    fn count(&mut self, element: usize, what: &'static str) -> Result<usize> {
        let count = self.u32(what)? as usize;
        match count.checked_mul(element) {
            Some(needed) if needed <= self.bytes.len() => Ok(count),
            _ => Err(invalid(format!("the cooked bytes end inside {what}"))),
        }
    }

    /// `count` elements read by `read`.
    fn list<T>(
        &mut self,
        element: usize,
        what: &'static str,
        mut read: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let count = self.count(element, what)?;
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            list.push(read(self)?);
        }
        Ok(list)
    }

    /// An optional index of one byte, [`NO_INDEX`] for `None`.
    fn index(&mut self, what: &'static str) -> Result<Option<u8>> {
        let index = self.u8(what)?;
        Ok((index != NO_INDEX).then_some(index))
    }
}

/// Refuses a `declared` dependency list (ascending, as the cooked header stores it) that is
/// not exactly the set of assets the bytes `named`, [`AssetId::NONE`] left out: a reference
/// missing from the header would be missing from every table pinned from it.
pub(crate) fn check_dependencies(
    declared: &[AssetId],
    named: impl IntoIterator<Item = AssetId>,
) -> Result<()> {
    let mut named: Vec<AssetId> = named.into_iter().filter(|id| !id.is_none()).collect();
    named.sort_unstable();
    named.dedup();
    if named == declared {
        Ok(())
    } else {
        Err(invalid(format!(
            "the header declares {} dependencies that are not the {} assets the bytes name",
            declared.len(),
            named.len()
        )))
    }
}

/// A [`Clip`]: the skeleton (`u64` asset ID), `duration_ticks` and `frames` (`u32`),
/// `looping` and `bones` (`u8`); then the samples, the root track (translation then yaw)
/// and the events (phase `u32`, key `u64`), each list counted.
pub(crate) fn decode_clip(bytes: &[u8]) -> Result<Clip> {
    let mut r = Reader::open::<Clip>(bytes)?;
    let clip = Clip {
        skeleton: r.asset("the clip's skeleton")?,
        duration_ticks: r.u32("the clip's duration")?,
        frames: r.u32("the clip's frame count")?,
        looping: r.bool("the clip's looping")?,
        bones: r.u8("the clip's bone count")?,
        samples: r.list(POSE_BYTES, "the clip's samples", |r| r.pose("a sample"))?,
        root: r.list(ROOT_BYTES, "the clip's root track", |r| {
            Ok(RootSample { translation: r.vec3("a root entry")?, yaw: r.scalar("a root entry")? })
        })?,
        events: r.list(EVENT_BYTES, "the clip's events", |r| {
            Ok(ClipEvent {
                phase: Phase(r.u32("an event")?),
                event: AnimEventKey(r.u64("an event")?),
            })
        })?,
    };
    r.finish()?;
    Ok(clip)
}

/// A [`StateMachine`]: `initial` (`u16`); the states (key, clip `u64`, speed, speed
/// parameter `u8`, `0xFF` for none); the transitions (`from` `u16`, `0xFFFF` for any
/// state, `to` and `blend_ticks` `u16`, `keep_phase` `u8`, `entry_phase` `u32`, then the
/// conditions, each a tag byte and its fields), each list counted.
pub(crate) fn decode_state_machine(bytes: &[u8]) -> Result<StateMachine> {
    let mut r = Reader::open::<StateMachine>(bytes)?;
    let initial = r.u16("the initial state")?;
    let states = r.list(STATE_BYTES, "the states", |r| {
        Ok(State {
            key: StateKey(r.u64("a state")?),
            clip: r.asset("a state")?,
            speed: r.scalar("a state")?,
            speed_param: r.index("a state")?,
        })
    })?;
    let transitions = r.list(TRANSITION_BYTES, "the transitions", |r| {
        let from = r.u16("a transition")?;
        Ok(Transition {
            from: (from != NO_STATE).then_some(from),
            to: r.u16("a transition")?,
            blend_ticks: r.u16("a transition")?,
            keep_phase: r.bool("a transition's keep_phase")?,
            entry_phase: Phase(r.u32("a transition")?),
            conditions: r.list(CONDITION_BYTES, "a transition's conditions", condition)?,
        })
    })?;
    r.finish()?;
    Ok(StateMachine { states, transitions, initial })
}

/// One [`Condition`]: its tag, then the variant's fields.
fn condition(r: &mut Reader<'_>) -> Result<Condition> {
    const WHAT: &str = "a condition";
    Ok(match r.u8(WHAT)? {
        FINISHED => Condition::Finished,
        PHASE_AT_LEAST => Condition::PhaseAtLeast(Phase(r.u32(WHAT)?)),
        TICKS_AT_LEAST => Condition::TicksAtLeast(r.u32(WHAT)?),
        ABOVE => Condition::Above { param: r.u8(WHAT)?, value: r.scalar(WHAT)? },
        BELOW => Condition::Below { param: r.u8(WHAT)?, value: r.scalar(WHAT)? },
        FLAG => Condition::Flag { bit: r.u8(WHAT)?, set: r.bool(WHAT)? },
        TRIGGER => Condition::Trigger(r.u8(WHAT)?),
        tag => return Err(invalid(format!("a condition has the unknown tag {tag}"))),
    })
}

/// A [`Skeleton`]: the bones (key, parent `u8`, `0xFF` for none, role `u8`, rest pose, then
/// a presence byte and the body) and the sockets (key, bone `u8`, offset), each list
/// counted.
pub(crate) fn decode_skeleton(bytes: &[u8]) -> Result<Skeleton> {
    let mut r = Reader::open::<Skeleton>(bytes)?;
    let bones = r.list(BONE_BYTES, "the bones", |r| {
        Ok(Bone {
            key: BoneKey(r.u64("a bone")?),
            parent: r.index("a bone")?,
            role: role(r.u8("a bone")?)?,
            rest: r.pose("a bone's rest pose")?,
            body: if r.bool("a bone's body presence")? { Some(body(r)?) } else { None },
        })
    })?;
    let sockets = r.list(SOCKET_BYTES, "the sockets", |r| {
        Ok(Socket {
            key: BoneKey(r.u64("a socket")?),
            bone: r.u8("a socket")?,
            offset: r.pose("a socket")?,
        })
    })?;
    r.finish()?;
    Ok(Skeleton { bones, sockets })
}

fn role(byte: u8) -> Result<BoneRole> {
    Ok(match byte {
        0 => BoneRole::Other,
        1 => BoneRole::Hips,
        2 => BoneRole::Spine,
        3 => BoneRole::Head,
        4 => BoneRole::HandLeft,
        5 => BoneRole::HandRight,
        6 => BoneRole::FootLeft,
        7 => BoneRole::FootRight,
        other => return Err(invalid(format!("a bone has the unknown role {other}"))),
    })
}

/// A [`BoneBody`]: the body (mass, the two dampings and the gravity scale, then
/// `body_type`, `locked_axes` and `can_sleep` as bytes); the collider (shape kind byte,
/// half extents, radius, offset, rotation, friction, restitution, density, impact
/// threshold, membership and filter `u32`, `sensor` and `events` bytes, material `u16`);
/// then a presence byte and the joint (kind byte, the two anchors, the axis, a presence
/// byte and the two limits).
fn body(r: &mut Reader<'_>) -> Result<BoneBody> {
    const WHAT: &str = "a bone's body";
    let mut rigid = RigidBody::dynamic();
    rigid.mass = r.scalar(WHAT)?;
    rigid.linear_damping = r.scalar(WHAT)?;
    rigid.angular_damping = r.scalar(WHAT)?;
    rigid.gravity_scale = r.scalar(WHAT)?;
    rigid.body_type = match r.u8(WHAT)? {
        0 => BodyType::Dynamic,
        1 => BodyType::Kinematic,
        2 => BodyType::Fixed,
        other => return Err(invalid(format!("a bone's body has the unknown type {other}"))),
    };
    rigid.locked_axes = r.u8(WHAT)?;
    rigid.can_sleep = r.bool(WHAT)?;

    const COLLIDER: &str = "a bone's collider";
    let kind = match r.u8(COLLIDER)? {
        0 => ShapeKind::Sphere,
        1 => ShapeKind::Capsule,
        2 => ShapeKind::Cuboid,
        other => return Err(invalid(format!("a bone's collider has the unknown shape {other}"))),
    };
    let mut shape = Shape::sphere(Scalar::ZERO);
    shape.kind = kind;
    shape.half_extents = r.vec3(COLLIDER)?;
    shape.radius = r.scalar(COLLIDER)?;
    let mut collider = Collider::new(shape);
    collider.offset = r.vec3(COLLIDER)?;
    collider.rotation = r.quat(COLLIDER)?;
    collider.friction = r.scalar(COLLIDER)?;
    collider.restitution = r.scalar(COLLIDER)?;
    collider.density = r.scalar(COLLIDER)?;
    collider.impact_threshold = r.scalar(COLLIDER)?;
    collider.groups = CollisionGroups::new(r.u32(COLLIDER)?, r.u32(COLLIDER)?);
    collider.sensor = r.bool(COLLIDER)?;
    collider.events = r.bool(COLLIDER)?;
    collider.material = r.u16(COLLIDER)?;

    const JOINT: &str = "a bone's joint";
    let joint = if r.bool(JOINT)? {
        let kind = match r.u8(JOINT)? {
            0 => JointKind::Fixed,
            1 => JointKind::Spherical,
            2 => JointKind::Revolute,
            3 => JointKind::Prismatic,
            other => return Err(invalid(format!("a bone's joint has the unknown kind {other}"))),
        };
        Some(BoneJoint {
            kind,
            anchor_parent: r.vec3(JOINT)?,
            anchor_bone: r.vec3(JOINT)?,
            axis: r.vec3(JOINT)?,
            limits: if r.bool(JOINT)? { Some((r.scalar(JOINT)?, r.scalar(JOINT)?)) } else { None },
        })
    } else {
        None
    };
    Ok(BoneBody { body: rigid, collider, joint })
}
