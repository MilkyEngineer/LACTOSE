//! The authored data the simulation animates with (architecture sections 14, 17 and 18):
//! clips, state machines and sim skeletons, as the three simulation asset kinds a region's
//! exact-version table carries, and the loaders that decode their cooked bytes (the byte
//! form is `README.md`'s, "Assets", and `codec.rs`'s): the runtime form every kernel reads,
//! the checks a loader runs on it, and the kind keys and format versions the cooker emits.

use std::borrow::Cow;

use lactose_asset_api::{Asset, AssetId, AssetKind, AssetRef, Loader, SimulationAsset};
use lactose_core::{Error, ErrorKind, Result};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{Collider, JointKind, RigidBody, ShapeKind, locked_axes};

use crate::codec;

use crate::components::{BoneIndex, BonePose, ParamIndex, RootMotion, StateIndex};
use crate::keys::{AnimEventKey, BoneKey, StateKey};
use crate::phase::Phase;

/// The most bones a sim skeleton has: hips, spine, chest, head, two upper and two lower
/// arms, two hands, two upper and two lower legs, two feet, and one to spare. Enough for
/// the ragdoll bodies and the hands, feet and head section 17 names, and small enough that
/// a [`SimPose`](crate::SimPose) of eight-byte scalars (904 bytes) stays under the ECS's
/// 1 KiB component warning. Sockets are offsets from a bone, not bones, so they do not
/// count against it.
pub const MAX_SIM_BONES: usize = 16;

/// How many parameter values a state machine reads from [`AnimParams`](crate::AnimParams).
pub const PARAM_SLOTS: usize = 8;

/// An animation clip cooked for one sim skeleton: `frames` samples of every bone's
/// parent-local pose, an optional root track, and the authored events. The sample rate is
/// the cooker's choice; `duration_ticks` says how long the clip takes at speed one, so a
/// clip may hold fewer samples than ticks.
#[derive(Clone, PartialEq, Debug)]
pub struct Clip {
    /// The skeleton whose bone order `samples` follow. A clip is evaluated only against
    /// this skeleton.
    pub skeleton: AssetRef<Skeleton>,
    /// The clip's length at speed one, in ticks; at least one.
    pub duration_ticks: u32,
    /// How many samples each bone has; at least two, or at least one for a looping clip.
    pub frames: u32,
    /// Whether the phase wraps (a cycle: sample `frames` is sample zero again) or saturates
    /// at [`Phase::END`] (a one-shot: sample `frames - 1` is the end).
    pub looping: bool,
    /// How many bones each frame holds: the skeleton's bone count.
    pub bones: u8,
    /// `frames * bones` parent-local poses, frame-major: frame `f`'s bone `b` is at
    /// `f * bones + b`.
    pub samples: Vec<BonePose>,
    /// The root's pose per frame in the character's space, or empty for a clip without root
    /// motion: `frames + 1` entries for a looping clip, the last being where the cycle ends
    /// (sample zero displaced by the cycle's total motion), `frames` for a one-shot. Entry
    /// zero is the identity, the character at the clip's start: the last entry's yaw is
    /// then the cycle's turn, which the wrap of [`root_motion`](Self::root_motion) adds.
    pub root: Vec<RootSample>,
    /// The authored events, ascending by phase.
    pub events: Vec<ClipEvent>,
}

/// One frame of a clip's root track.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RootSample {
    /// Where the root is, in the character's space at the clip's start.
    pub translation: Vec3,
    /// How far the root has turned about `+Y`, in radians, positive to the left.
    pub yaw: Scalar,
}

/// An authored event at a phase of a clip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClipEvent {
    /// Where in the clip.
    pub phase: Phase,
    /// Which event.
    pub event: AnimEventKey,
}

impl Clip {
    /// Bone `bone`'s parent-local pose at `phase`: the two samples the phase lies between,
    /// blended by the fraction between them ([`BonePose::blend`]). The sample position is
    /// `phase * intervals / 2^32` in integer arithmetic, with `intervals` equal to `frames`
    /// for a looping clip (the sample after the last is sample zero) and `frames - 1`
    /// otherwise, the fraction one `lactose_math` division; [`Phase::END`] on a one-shot
    /// samples the last frame exactly, as does a phase whose position falls on a frame (a
    /// zero fraction), which is that frame's sample unblended. A `bone` at or past `bones`
    /// is the identity.
    #[must_use]
    pub fn sample(&self, phase: Phase, bone: BoneIndex) -> BonePose {
        self.pose_at(self.cursor(phase), bone)
    }

    /// What the root moved by between phase `from` and phase `to` of this tick's advance, in
    /// the character's space at `from`. The track is in the clip-start space, so the
    /// difference is turned back by the yaw already turned: with `track(p)` and `yaw(p)` the
    /// track sampled at `p` by the rule of [`sample`](Self::sample), `R(a)` the rotation by
    /// `a` about `+Y`, `end` the track's last entry and `total` its yaw,
    ///
    /// - without a wrap (`to >= from`): `translation = R(-yaw(from)) * (track(to) -
    ///   track(from))` and `yaw = yaw(to) - yaw(from)`;
    /// - across a wrap (`to < from`, a looping clip): `translation = R(-yaw(from)) *
    ///   (end - track(from)) + R(total - yaw(from)) * (track(to) - track(0))` and `yaw =
    ///   total - yaw(from) + yaw(to)`.
    ///
    /// So a looping walk's motion sums to its cycle's total over one cycle at any rate,
    /// turned as the character turns. Zero for a clip without a root track or when
    /// `from == to`.
    #[must_use]
    pub fn root_motion(&self, from: Phase, to: Phase) -> RootMotion {
        if self.root.is_empty() || from == to {
            return RootMotion::ZERO;
        }
        let start = self.track(from);
        let reached = self.track(to);
        let turn_back = Quat::from_rotation_y(-start.yaw);
        if to >= from || !self.looping {
            return RootMotion {
                translation: turn_back * (reached.translation - start.translation),
                yaw: reached.yaw - start.yaw,
            };
        }
        let first = self.root_entry(0);
        let end = self.root_entry(self.root.len() - 1);
        let total = end.yaw;
        let turned = Quat::from_rotation_y(total - start.yaw);
        RootMotion {
            translation: turn_back * (end.translation - start.translation)
                + turned * (reached.translation - first.translation),
            yaw: total - start.yaw + reached.yaw,
        }
    }

    /// The two frames `phase` lies between and the fraction between them, by the rule of
    /// [`sample`](Self::sample). Frame `b` is `frames` past the last frame of a looping
    /// clip: sample zero for a pose, the cycle's end for the root track.
    pub(crate) fn cursor(&self, phase: Phase) -> Cursor {
        let intervals = if self.looping { self.frames } else { self.frames.saturating_sub(1) };
        if intervals == 0 || (!self.looping && phase == Phase::END) {
            let last = intervals as usize;
            return Cursor { a: last, b: last, t: None };
        }
        // Below 2^64: both factors are below 2^32.
        let position = u64::from(phase.0) * u64::from(intervals);
        let a = (position >> 32) as usize;
        let fraction = position & (Phase::UNITS - 1);
        if fraction == 0 {
            return Cursor { a, b: a, t: None };
        }
        let t = Scalar::from_u64(fraction) / Scalar::from_u64(Phase::UNITS);
        Cursor { a, b: a + 1, t: Some(t) }
    }

    /// Bone `bone`'s parent-local pose at `cursor`: the identity past the bones, and for a
    /// frame the samples do not hold (a clip `validate` refuses).
    pub(crate) fn pose_at(&self, cursor: Cursor, bone: BoneIndex) -> BonePose {
        let bones = usize::from(self.bones);
        let bone = usize::from(bone);
        if bone >= bones {
            return BonePose::IDENTITY;
        }
        let frames = self.frames as usize;
        let at = |frame: usize| {
            let frame = if frame >= frames { 0 } else { frame };
            self.samples.get(frame * bones + bone).copied().unwrap_or(BonePose::IDENTITY)
        };
        match cursor.t {
            None => at(cursor.a),
            Some(t) => at(cursor.a).blend(at(cursor.b), t),
        }
    }

    /// The root track at `phase`, sampled by the rule of [`sample`](Self::sample).
    fn track(&self, phase: Phase) -> RootSample {
        let cursor = self.cursor(phase);
        let a = self.root_entry(cursor.a);
        match cursor.t {
            None => a,
            Some(t) => {
                let b = self.root_entry(cursor.b);
                RootSample {
                    translation: a.translation.lerp(b.translation, t),
                    yaw: a.yaw.lerp(b.yaw, t),
                }
            }
        }
    }

    /// Entry `index` of the root track, or zero past it (a track `validate` refuses).
    fn root_entry(&self, index: usize) -> RootSample {
        self.root
            .get(index)
            .copied()
            .unwrap_or(RootSample { translation: Vec3::ZERO, yaw: Scalar::ZERO })
    }

    /// The checks the loader runs: `duration_ticks` at least one; `frames` at least two, or
    /// one when looping; `samples.len() == frames * bones`; `root` empty or of the length
    /// stated on the field, its entry zero the identity (no translation, no yaw); every
    /// rotation a unit quaternion; every scalar finite; events ascending by phase.
    /// `InvalidData` otherwise, naming the first failure.
    pub fn validate(&self) -> Result<()> {
        if self.duration_ticks == 0 {
            return Err(invalid("a clip lasts at least one tick"));
        }
        if self.frames < 2 && !(self.looping && self.frames == 1) {
            return Err(invalid("a clip has two frames, or one when it loops"));
        }
        let samples = u64::from(self.frames) * u64::from(self.bones);
        if self.samples.len() as u64 != samples {
            return Err(invalid(format!(
                "a clip of {} frames and {} bones holds {samples} samples, not {}",
                self.frames,
                self.bones,
                self.samples.len()
            )));
        }
        let root = u64::from(self.frames) + u64::from(self.looping);
        if !self.root.is_empty() && self.root.len() as u64 != root {
            return Err(invalid(format!(
                "a clip's root track holds {root} entries or none, not {}",
                self.root.len()
            )));
        }
        for (index, sample) in self.samples.iter().enumerate() {
            pose_is_valid(sample).map_err(|what| invalid(format!("sample {index}: {what}")))?;
        }
        for (index, entry) in self.root.iter().enumerate() {
            if !entry.translation.is_finite() || !entry.yaw.is_finite() {
                return Err(invalid(format!("root entry {index}: a scalar is not finite")));
            }
        }
        // The wrap adds the last entry's yaw as the cycle's turn, which holds only from an
        // unturned entry zero; the track is in the character's space at the clip's start.
        if self
            .root
            .first()
            .is_some_and(|first| first.translation != Vec3::ZERO || first.yaw != Scalar::ZERO)
        {
            return Err(invalid("root entry 0: the track does not start at the identity"));
        }
        let descending = self.events.windows(2).position(|pair| pair[1].phase < pair[0].phase);
        if let Some(index) = descending {
            return Err(invalid(format!(
                "event {}: the events are not ascending by phase",
                index + 1
            )));
        }
        Ok(())
    }
}

impl Asset for Clip {
    const KIND: AssetKind = AssetKind::from_name("anim::clip");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Clip {}

/// A state machine: states that each play a clip, and transitions between them tried in
/// order, the first whose conditions all hold taken.
#[derive(Clone, PartialEq, Debug)]
pub struct StateMachine {
    /// The states, by [`StateIndex`]; fewer than `u16::MAX`.
    pub states: Vec<State>,
    /// The transitions, in priority order.
    pub transitions: Vec<Transition>,
    /// The state the first advance enters.
    pub initial: StateIndex,
}

/// One state: a clip played at a speed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct State {
    /// The state's key.
    pub key: StateKey,
    /// The clip the state plays.
    pub clip: AssetRef<Clip>,
    /// The playback speed, one for the clip's authored rate; finite and not negative.
    pub speed: Scalar,
    /// A parameter whose value multiplies `speed` (a walk cycle scaled by the character's
    /// speed, which the game quantises first, D124: `AnimParams::quantise`), or `None`. A
    /// product that is not finite, negative or zero holds the state (`Phase::rate`).
    pub speed_param: Option<ParamIndex>,
}

/// A transition from one state (or any state) to another, or from a state to itself: a
/// restart, which enters the state again (entered, the phase set or kept, a blend between
/// the clip at its two phases if `blend_ticks` is set).
#[derive(Clone, PartialEq, Debug)]
pub struct Transition {
    /// The state the transition leaves, or `None` for any state but `to` (an any-state
    /// transition never restarts its target; a restart names `from` equal to `to`).
    pub from: Option<StateIndex>,
    /// The state it enters.
    pub to: StateIndex,
    /// The conditions, all of which must hold.
    pub conditions: Vec<Condition>,
    /// How many ticks the outgoing pose blends out over; zero for a cut.
    pub blend_ticks: u16,
    /// Whether the new state starts at the old state's phase (a walk cycle into a run cycle)
    /// rather than at `entry_phase`.
    pub keep_phase: bool,
    /// Where the new state starts, unless `keep_phase`.
    pub entry_phase: Phase,
}

/// One condition of a [`Transition`], over the state's [`AnimState`](crate::AnimState) and
/// the character's [`AnimParams`](crate::AnimParams) as they are when `anim::advance`
/// runs, before this tick's phase advance.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Condition {
    /// The state's one-shot clip has reached its end.
    Finished,
    /// The state's phase is at or past this phase.
    PhaseAtLeast(Phase),
    /// The state has run at least this many ticks.
    TicksAtLeast(u32),
    /// Parameter `param` is above `value`.
    Above {
        /// Which value.
        param: ParamIndex,
        /// The threshold, exclusive.
        value: Scalar,
    },
    /// Parameter `param` is below `value`.
    Below {
        /// Which value.
        param: ParamIndex,
        /// The threshold, exclusive.
        value: Scalar,
    },
    /// Flag `bit` is set (or clear, when `set` is false).
    Flag {
        /// Which flag.
        bit: u8,
        /// Whether it must be set.
        set: bool,
    },
    /// Trigger `bit` is set; the advance clears every trigger after its transitions.
    Trigger(u8),
}

impl StateMachine {
    /// The state at `index`, or `None` past the states.
    #[must_use]
    pub fn state(&self, index: StateIndex) -> Option<&State> {
        self.states.get(usize::from(index))
    }

    /// The index of the state with `key`, or `None`.
    #[must_use]
    pub fn find(&self, key: StateKey) -> Option<StateIndex> {
        self.states
            .iter()
            .position(|state| state.key == key)
            .and_then(|index| StateIndex::try_from(index).ok())
    }

    /// The checks the loader runs: fewer than `u16::MAX` states and at least one; `initial`
    /// and every transition's `from` and `to` in range (`from` equal to `to` is a restart
    /// and allowed); every `speed_param` and every condition's `param` below
    /// [`PARAM_SLOTS`], every flag and trigger bit below 32; no two states with one key;
    /// every speed finite and not negative. `InvalidData` otherwise, naming the first
    /// failure.
    pub fn validate(&self) -> Result<()> {
        let states = self.states.len();
        if states == 0 || states >= usize::from(u16::MAX) {
            return Err(invalid(format!(
                "a state machine has at least one state and fewer than {}, not {states}",
                u16::MAX
            )));
        }
        if usize::from(self.initial) >= states {
            return Err(invalid(format!("the initial state {} is past the states", self.initial)));
        }
        // The first state whose key an earlier state has, found by sorting (a cooked
        // machine holds up to 65,534 states): in each run of one key, ascending by index,
        // the second is the first repeat.
        let mut keys: Vec<(StateKey, usize)> =
            self.states.iter().enumerate().map(|(index, state)| (state.key, index)).collect();
        keys.sort_unstable();
        let repeat = keys.windows(2).filter(|pair| pair[0].0 == pair[1].0).map(|pair| pair[1].1);
        let first_repeat = repeat.min();
        for (index, state) in self.states.iter().enumerate() {
            if !state.speed.is_finite() || state.speed < Scalar::ZERO {
                return Err(invalid(format!("state {index}: the speed is negative or not finite")));
            }
            if state.speed_param.is_some_and(|param| usize::from(param) >= PARAM_SLOTS) {
                return Err(invalid(format!(
                    "state {index}: the speed parameter is past the slots"
                )));
            }
            if first_repeat == Some(index) {
                return Err(invalid(format!("state {index}: another state has its key")));
            }
        }
        for (index, transition) in self.transitions.iter().enumerate() {
            transition
                .check(states)
                .map_err(|what| invalid(format!("transition {index}: {what}")))?;
        }
        Ok(())
    }
}

impl Asset for StateMachine {
    const KIND: AssetKind = AssetKind::from_name("anim::state_machine");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for StateMachine {}

/// A sim skeleton: the bones the simulation evaluates (at most [`MAX_SIM_BONES`], a parent
/// always before its children, bone zero the root bone with no parent), with each bone's
/// role, rest pose and optional physics body, and the sockets that hang off them.
#[derive(Clone, PartialEq, Debug)]
pub struct Skeleton {
    /// The bones, by [`BoneIndex`], parents before children.
    pub bones: Vec<Bone>,
    /// The sockets, by index.
    pub sockets: Vec<Socket>,
}

/// One bone of a sim skeleton.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bone {
    /// The bone's key.
    pub key: BoneKey,
    /// The parent bone, which precedes this one, or `None` for the root bone.
    pub parent: Option<BoneIndex>,
    /// What the bone is to the game.
    pub role: BoneRole,
    /// The bone's parent-local pose when no clip plays.
    pub rest: BonePose,
    /// The physics body a bone row carries, or `None` for a bone that is evaluated only.
    pub body: Option<BoneBody>,
}

/// What a bone is to the game, so hands, feet and the head are found without a name
/// (architecture section 17). At most one bone of a skeleton has each role other than
/// `Other`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum BoneRole {
    /// A bone with no engine-known role.
    Other = 0,
    /// The pelvis: the root of the ragdoll.
    Hips = 1,
    /// The spine or chest.
    Spine = 2,
    /// The head.
    Head = 3,
    /// The left hand.
    HandLeft = 4,
    /// The right hand.
    HandRight = 5,
    /// The left foot.
    FootLeft = 6,
    /// The right foot.
    FootRight = 7,
}

/// The physics body of a bone row (`README.md`, "Bones and ragdolls"): its body, its
/// collider in the bone's space, and the joint to the nearest ancestor with a body.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BoneBody {
    /// The body's mass, damping and sleep settings; [`Bones::spawn`](crate::Bones::spawn)
    /// makes it kinematic and [`Bones::set_limp`](crate::Bones::set_limp) dynamic, whatever
    /// `body_type` says here.
    pub body: RigidBody,
    /// The collider, placed in the bone's space. Its `groups` follow the bone rule
    /// (`README.md`, "Bones and ragdolls"), which is on the skeleton's bone group set, not
    /// on one collider: over the skeleton's bodies, no `filter` meets any `membership`,
    /// which [`Skeleton::validate`] checks, so no two of its bones touch; the game gives
    /// every skeleton's bones the one bone group set, so no two characters' bones touch;
    /// and a bone's `filter` excludes the character capsule's `membership`, so no bone
    /// touches a capsule. What hits a bone is a query, a sensor or a weapon collider in a
    /// group the bone's filter admits and whose filter admits the bone's membership.
    pub collider: Collider,
    /// The joint to the nearest ancestor bone that has a body, or `None` for the ragdoll's
    /// root body.
    pub joint: Option<BoneJoint>,
}

/// A ragdoll joint between a bone's body and its nearest ancestor's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BoneJoint {
    /// What the joint constrains.
    pub kind: JointKind,
    /// The anchor in the ancestor's space.
    pub anchor_parent: Vec3,
    /// The anchor in the bone's space.
    pub anchor_bone: Vec3,
    /// The axis of a revolute or prismatic joint, in the ancestor's space.
    pub axis: Vec3,
    /// The limits, if limited: see `Joint::with_limits` and the backend's verification
    /// item on limited joints in `crates/physics-api/README.md`.
    pub limits: Option<(Scalar, Scalar)>,
}

/// A point hung off a bone: a weapon grip, a shield mount, a camera target. An offset, not
/// a bone, so it costs no evaluation and does not count against [`MAX_SIM_BONES`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Socket {
    /// The socket's key.
    pub key: BoneKey,
    /// The bone it hangs off.
    pub bone: BoneIndex,
    /// Its pose in the bone's space.
    pub offset: BonePose,
}

impl Skeleton {
    /// The index of the bone with `key`, or `None`.
    #[must_use]
    pub fn find(&self, key: BoneKey) -> Option<BoneIndex> {
        self.bones
            .iter()
            .position(|bone| bone.key == key)
            .and_then(|index| BoneIndex::try_from(index).ok())
    }

    /// The index of the bone with `role`, or `None`; `None` for [`BoneRole::Other`].
    #[must_use]
    pub fn bone_of(&self, role: BoneRole) -> Option<BoneIndex> {
        if role == BoneRole::Other {
            return None;
        }
        self.bones
            .iter()
            .position(|bone| bone.role == role)
            .and_then(|index| BoneIndex::try_from(index).ok())
    }

    /// The index of the socket with `key`, or `None`.
    #[must_use]
    pub fn socket(&self, key: BoneKey) -> Option<usize> {
        self.sockets.iter().position(|socket| socket.key == key)
    }

    /// The checks the loader runs: at least one bone and at most [`MAX_SIM_BONES`]; bone
    /// zero without a parent and every other bone's parent before it; no two bones with one
    /// key and no two with one role other than `Other`; every socket on a bone in range; a
    /// joint only on a bone that has a body, and a body's joint reaching an ancestor with a
    /// body (the ragdoll's root body alone has none); the OR of every body's
    /// `groups.membership` and the OR of every body's `groups.filter` sharing no bit (the
    /// bone rule, [`BoneBody::collider`]);
    /// every rest and socket rotation a unit quaternion and every scalar finite; and every
    /// [`BoneBody`] one the physics backend accepts, so no bone's body is refused at every
    /// sync: the mass and both dampings non-negative; `locked_axes` within
    /// `locked_axes::TRANSLATION | locked_axes::ROTATION`; a sphere's radius positive, a
    /// capsule's radius positive and its half height (`half_extents.y`) non-negative, a
    /// cuboid's three half extents positive; the collider's rotation a unit quaternion;
    /// friction, restitution, density and the impact threshold non-negative; a revolute or
    /// prismatic joint's axis non-zero; a limited joint's `min` at most its `max`. These
    /// are the rules of `lactose-physics-rapier`'s conversion, the backend `physics-api`'s
    /// validity statement ("a NaN, a non-unit quaternion, a negative extent") leaves to.
    /// `InvalidData` otherwise, naming the first failure.
    pub fn validate(&self) -> Result<()> {
        let bones = self.bones.len();
        if bones == 0 || bones > MAX_SIM_BONES {
            return Err(invalid(format!(
                "a skeleton has at least one bone and at most {MAX_SIM_BONES}, not {bones}"
            )));
        }
        let (mut membership, mut filter) = (0u32, 0u32);
        for (index, bone) in self.bones.iter().enumerate() {
            let fail = |what: &str| invalid(format!("bone {index}: {what}"));
            match (index, bone.parent) {
                (0, Some(_)) => return Err(fail("the root bone has a parent")),
                (0, None) => {}
                (_, None) => return Err(fail("only bone zero has no parent")),
                (_, Some(parent)) if usize::from(parent) >= index => {
                    return Err(fail("the parent does not precede the bone"));
                }
                (_, Some(_)) => {}
            }
            if self.bones[..index].iter().any(|other| other.key == bone.key) {
                return Err(fail("another bone has its key"));
            }
            if bone.role != BoneRole::Other
                && self.bones[..index].iter().any(|other| other.role == bone.role)
            {
                return Err(fail("another bone has its role"));
            }
            pose_is_valid(&bone.rest).map_err(|what| fail(&format!("the rest pose: {what}")))?;
            let Some(body) = &bone.body else { continue };
            body_is_valid(body).map_err(fail)?;
            if body.joint.is_some() && self.body_above(index).is_none() {
                return Err(fail("the joint reaches no ancestor with a body"));
            }
            membership |= body.collider.groups.membership;
            filter |= body.collider.groups.filter;
        }
        if membership & filter != 0 {
            return Err(invalid(format!(
                "the bones' memberships {membership:#x} meet their filters {filter:#x}, so two \
                 bones of the skeleton would touch"
            )));
        }
        for (index, socket) in self.sockets.iter().enumerate() {
            if usize::from(socket.bone) >= bones {
                return Err(invalid(format!("socket {index}: the bone is past the bones")));
            }
            pose_is_valid(&socket.offset)
                .map_err(|what| invalid(format!("socket {index}: the offset: {what}")))?;
        }
        Ok(())
    }

    /// The nearest ancestor of bone `index` that has a body, walking at most as many
    /// parents as there are bones, so a skeleton `validate` refuses cannot loop it.
    pub(crate) fn body_above(&self, index: usize) -> Option<usize> {
        let mut bone = self.bones.get(index)?;
        for _ in 0..self.bones.len() {
            let parent = usize::from(bone.parent?);
            bone = self.bones.get(parent)?;
            if bone.body.is_some() {
                return Some(parent);
            }
        }
        None
    }
}

impl Transition {
    /// Whether every index and bit of the transition is in range for a machine of `states`
    /// states: what [`StateMachine::validate`] checks of it, and what the advance checks
    /// before it tries it.
    pub(crate) fn check(&self, states: usize) -> core::result::Result<(), &'static str> {
        if self.from.is_some_and(|from| usize::from(from) >= states) {
            return Err("the state it leaves is past the states");
        }
        if usize::from(self.to) >= states {
            return Err("the state it enters is past the states");
        }
        for condition in &self.conditions {
            match *condition {
                Condition::Above { param, .. } | Condition::Below { param, .. }
                    if usize::from(param) >= PARAM_SLOTS =>
                {
                    return Err("a condition's parameter is past the slots");
                }
                Condition::Flag { bit, .. } | Condition::Trigger(bit) if bit >= 32 => {
                    return Err("a condition's bit is past the word");
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Where a phase falls between two frames of a clip: frames `a` and `b` and the fraction
/// between them, or `None` when the phase is on frame `a`, which it then samples exactly.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Cursor {
    a: usize,
    b: usize,
    t: Option<Scalar>,
}

/// The tolerance of the unit-quaternion check on the squared length: the one the physics
/// backend applies to the rotations it is handed.
const UNIT_TOLERANCE: Scalar = Scalar::from_f64(1e-6);

/// Whether `pose` is finite with a unit rotation.
fn pose_is_valid(pose: &BonePose) -> core::result::Result<(), &'static str> {
    if !pose.translation.is_finite() || !pose.rotation.is_finite() {
        return Err("a scalar is not finite");
    }
    if (pose.rotation.length_squared() - Scalar::ONE).abs() >= UNIT_TOLERANCE {
        return Err("the rotation is not a unit quaternion");
    }
    Ok(())
}

/// Whether the physics backend accepts `body`: every scalar finite, then the rules
/// [`Skeleton::validate`] lists, which are `lactose-physics-rapier`'s `valid_body`,
/// `valid_collider`, `valid_shape` and `valid_joint` over the rows `Bones::spawn` writes.
fn body_is_valid(body: &BoneBody) -> core::result::Result<(), &'static str> {
    body_is_finite(body)?;
    let non_negative = |value: Scalar| value >= Scalar::ZERO;
    let positive = |value: Scalar| value > Scalar::ZERO;
    let rigid = &body.body;
    if ![rigid.mass, rigid.linear_damping, rigid.angular_damping].into_iter().all(non_negative) {
        return Err("the body's mass or damping is negative");
    }
    if rigid.locked_axes & !(locked_axes::TRANSLATION | locked_axes::ROTATION) != 0 {
        return Err("the body's locked axes name an unknown bit");
    }
    let collider = &body.collider;
    let shape = &collider.shape;
    let extents = match shape.kind {
        ShapeKind::Sphere => positive(shape.radius),
        ShapeKind::Capsule => positive(shape.radius) && non_negative(shape.half_extents.y),
        ShapeKind::Cuboid => shape.half_extents.to_array().into_iter().all(positive),
    };
    if !extents {
        return Err("the collider's shape has a zero or negative extent");
    }
    if (collider.rotation.length_squared() - Scalar::ONE).abs() >= UNIT_TOLERANCE {
        return Err("the collider's rotation is not a unit quaternion");
    }
    let coefficients =
        [collider.friction, collider.restitution, collider.density, collider.impact_threshold];
    if !coefficients.into_iter().all(non_negative) {
        return Err("the collider's friction, restitution, density or threshold is negative");
    }
    let Some(joint) = &body.joint else { return Ok(()) };
    let axial = matches!(joint.kind, JointKind::Revolute | JointKind::Prismatic);
    if axial && joint.axis.try_normalize().is_none() {
        return Err("the joint's axis is zero");
    }
    if joint.limits.is_some_and(|(min, max)| min > max) {
        return Err("the joint's lower limit is above its upper");
    }
    Ok(())
}

/// Whether every scalar of `body` is finite.
fn body_is_finite(body: &BoneBody) -> core::result::Result<(), &'static str> {
    let rigid = &body.body;
    let collider = &body.collider;
    let scalars = [
        rigid.mass,
        rigid.linear_damping,
        rigid.angular_damping,
        rigid.gravity_scale,
        collider.shape.radius,
        collider.friction,
        collider.restitution,
        collider.density,
        collider.impact_threshold,
    ];
    let vectors = [collider.shape.half_extents, collider.offset];
    let joint = body.joint.is_none_or(|joint| {
        joint.anchor_parent.is_finite()
            && joint.anchor_bone.is_finite()
            && joint.axis.is_finite()
            && joint.limits.is_none_or(|(min, max)| min.is_finite() && max.is_finite())
    });
    if scalars.iter().all(|scalar| scalar.is_finite())
        && vectors.iter().all(|vector| vector.is_finite())
        && collider.rotation.is_finite()
        && joint
    {
        Ok(())
    } else {
        Err("a scalar of the body is not finite")
    }
}

/// An `InvalidData` error.
pub(crate) fn invalid(message: impl Into<Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

impl Asset for Skeleton {
    const KIND: AssetKind = AssetKind::from_name("anim::skeleton");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Skeleton {}

/// The loader of [`Clip`]s: a pure decoder of the cooked bytes, bit-identical on every
/// host, which is the promise [`SimulationAsset`] makes; it validates what it decodes with
/// [`Clip::validate`]. The one dependency is the clip's skeleton. `app` registers it with
/// the store through `ErasedLoader::new`. Each of the three loaders refuses bytes in
/// another format or for another scalar width (`VersionMismatch`), bytes that are
/// truncated or run on past the asset, and a `dependencies` list other than the set of
/// assets the bytes name (`InvalidData`, `README.md`, "Assets", gives the byte form).
#[derive(Clone, Copy, Default, Debug)]
pub struct ClipLoader;

impl Loader for ClipLoader {
    type Asset = Clip;

    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Clip> {
        let clip = codec::decode_clip(bytes)?;
        codec::check_dependencies(dependencies, [clip.skeleton.id()])?;
        clip.validate()?;
        Ok(clip)
    }
}

/// The loader of [`StateMachine`]s: a pure decoder, validating with
/// [`StateMachine::validate`]. The dependencies are the states' clips, as for
/// [`ClipLoader`].
#[derive(Clone, Copy, Default, Debug)]
pub struct StateMachineLoader;

impl Loader for StateMachineLoader {
    type Asset = StateMachine;

    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<StateMachine> {
        let machine = codec::decode_state_machine(bytes)?;
        let clips = machine.states.iter().map(|state| state.clip.id());
        codec::check_dependencies(dependencies, clips)?;
        machine.validate()?;
        Ok(machine)
    }
}

/// The loader of [`Skeleton`]s: a pure decoder, validating with [`Skeleton::validate`]. No
/// dependencies, as for [`ClipLoader`].
#[derive(Clone, Copy, Default, Debug)]
pub struct SkeletonLoader;

impl Loader for SkeletonLoader {
    type Asset = Skeleton;

    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Skeleton> {
        let skeleton = codec::decode_skeleton(bytes)?;
        codec::check_dependencies(dependencies, [])?;
        skeleton.validate()?;
        Ok(skeleton)
    }
}
