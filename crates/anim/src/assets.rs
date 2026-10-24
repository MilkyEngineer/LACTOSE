//! The authored data the simulation animates with (architecture sections 14, 17 and 18):
//! clips, state machines and sim skeletons, as the three simulation asset kinds a region's
//! exact-version table carries, and the loaders that decode their cooked bytes. The byte
//! form is the implementation lane's, fixed together with the cooker and recorded in
//! `README.md` then; what is fixed here is the runtime form every kernel reads and the
//! kind keys and format versions the cooker emits.

use lactose_asset_api::{Asset, AssetId, AssetKind, AssetRef, Loader, SimulationAsset};
use lactose_core::Result;
use lactose_math::{Scalar, Vec3};
use lactose_physics_api::{Collider, JointKind, RigidBody};

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
    /// (sample zero displaced by the cycle's total motion), `frames` for a one-shot.
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
    /// samples the last frame exactly. A `bone` at or past `bones` is the identity.
    #[must_use]
    pub fn sample(&self, phase: Phase, bone: BoneIndex) -> BonePose {
        todo!()
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
        todo!()
    }

    /// The checks the loader runs: `duration_ticks` at least one; `frames` at least two, or
    /// one when looping; `samples.len() == frames * bones`; `root` empty or of the length
    /// stated on the field; every rotation a unit quaternion; every scalar finite; events
    /// ascending by phase. `InvalidData` otherwise, naming the first failure.
    pub fn validate(&self) -> Result<()> {
        todo!()
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
        todo!()
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
    /// every rest rotation a unit quaternion and every scalar finite. `InvalidData`
    /// otherwise, naming the first failure.
    pub fn validate(&self) -> Result<()> {
        todo!()
    }
}

impl Asset for Skeleton {
    const KIND: AssetKind = AssetKind::from_name("anim::skeleton");
    const FORMAT_VERSION: u32 = 1;
}

impl SimulationAsset for Skeleton {}

/// The loader of [`Clip`]s: a pure decoder of the cooked bytes, bit-identical on every
/// host, which is the promise [`SimulationAsset`] makes; it validates what it decodes with
/// [`Clip::validate`]. The one dependency is the clip's skeleton. `app` registers it with
/// the store through `ErasedLoader::new`.
#[derive(Clone, Copy, Default, Debug)]
pub struct ClipLoader;

impl Loader for ClipLoader {
    type Asset = Clip;

    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Clip> {
        todo!()
    }
}

/// The loader of [`StateMachine`]s: a pure decoder, validating with
/// [`StateMachine::validate`]. The dependencies are the states' clips.
#[derive(Clone, Copy, Default, Debug)]
pub struct StateMachineLoader;

impl Loader for StateMachineLoader {
    type Asset = StateMachine;

    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<StateMachine> {
        todo!()
    }
}

/// The loader of [`Skeleton`]s: a pure decoder, validating with [`Skeleton::validate`]. No
/// dependencies.
#[derive(Clone, Copy, Default, Debug)]
pub struct SkeletonLoader;

impl Loader for SkeletonLoader {
    type Asset = Skeleton;

    fn load(&self, bytes: &[u8], dependencies: &[AssetId]) -> Result<Skeleton> {
        todo!()
    }
}
