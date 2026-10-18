//! The components of an animated character and of its bone rows (architecture sections 6,
//! 15.1 and 17): what the game writes ([`AnimRig`], [`AnimParams`]), what the state machine
//! keeps ([`AnimState`]), what this crate writes every tick ([`RootMotion`], [`SimPose`]),
//! and the rows of a ragdoll ([`Bones`], [`BoneLink`]).
//!
//! Every component is its scalars first, then a tail whose size is a multiple of eight
//! bytes, so the layout holds whichever width the milestone 1 precision test picks for
//! [`Scalar`] without a padding change; `lib.rs` asserts the sizes for both widths. Every
//! value that crosses a tick boundary is an integer, a `Scalar` or a `lactose_math` vector
//! or quaternion: nothing here is a float of the platform library.

use lactose_asset_api::AssetRef;
use lactose_core::{EntityRef, PlainArray, StableKey};
use lactose_ecs::Commands;
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::Transform;
use lactose_reflect::{Padding, ZeroByte, component, value};

use crate::assets::{Clip, MAX_SIM_BONES, PARAM_SLOTS, Skeleton, StateMachine};
use crate::phase::Phase;

/// `N` zero bytes, in a constant.
const fn pad<const N: usize>() -> Padding<N> {
    PlainArray([ZeroByte::Zero; N])
}

/// The index of a state in its [`StateMachine`]'s `states`.
pub type StateIndex = u16;

/// The index of a bone in its [`Skeleton`]'s `bones`, below [`MAX_SIM_BONES`].
pub type BoneIndex = u8;

/// The index of a parameter value in [`AnimParams::values`], below [`PARAM_SLOTS`].
pub type ParamIndex = u8;

/// A bone's placement: relative to its parent bone in a clip sample and in a skeleton's
/// rest pose, relative to the character's root in a [`SimPose`]. A translation and a unit
/// rotation, no scale (a sim skeleton is never scaled; the render skeleton's scale is
/// presentation's).
#[value]
#[derive(PartialEq, Debug)]
pub struct BonePose {
    /// Where the bone's origin is, in the parent's or the root's space.
    #[field(id = 1)]
    pub translation: Vec3,
    /// How the bone is turned, in the same space: a unit quaternion.
    #[field(id = 2)]
    pub rotation: Quat,
}

impl BonePose {
    /// At the parent's origin, unrotated.
    pub const IDENTITY: Self = Self { translation: Vec3::ZERO, rotation: Quat::IDENTITY };

    /// A pose from its parts.
    #[must_use]
    pub const fn new(translation: Vec3, rotation: Quat) -> Self {
        Self { translation, rotation }
    }

    /// `child` placed in the space of `self`: the translation is
    /// `self.rotation * child.translation + self.translation`, the rotation
    /// `self.rotation * child.rotation`. This is the one composition the skeleton
    /// evaluation uses, parent before child, so a pose is a left fold over the bone chain.
    #[must_use]
    pub fn compose(self, child: Self) -> Self {
        Self {
            translation: self.rotation * child.translation + self.translation,
            rotation: self.rotation * child.rotation,
        }
    }

    /// This root-local pose placed by `root`: the simulation transform of the bone in world
    /// space, `root.transform_point(translation)` and `root.rotation * rotation`.
    #[must_use]
    pub fn in_world(self, root: &Transform) -> Transform {
        Transform::new(root.transform_point(self.translation), root.rotation * self.rotation)
    }

    /// `self` blended toward `other` by `t` in `[0, 1]`: the translation interpolated
    /// linearly (`Vec3::lerp`), the rotation by the normalised linear interpolation along the
    /// shorter arc (`Quat::nlerp`), never a slerp, so a blend costs no transcendental and the
    /// same bits come out on every target.
    #[must_use]
    pub fn blend(self, other: Self, t: Scalar) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.nlerp(other.rotation, t),
        }
    }
}

/// Which authored data a character animates with: its state machine and its sim skeleton.
/// Written by the game when the character is spawned, and rewritten to change either (a
/// rewrite that changes the machine makes the next `anim::advance` treat the state as
/// [`AnimState::UNSET`] if the state index is out of range). Both assets must be in the
/// region's exact-version table (architecture section 14): a rig whose assets the table
/// lacks is skipped, tick after tick, until they enter (`README.md`, "Assets").
#[component(key = "anim::rig", persistent, replicated, extracted)]
#[derive(PartialEq, Eq, Debug)]
pub struct AnimRig {
    /// The state machine that drives the character.
    #[field(id = 1)]
    pub machine: AssetRef<StateMachine>,
    /// The sim skeleton the machine's clips were cooked for.
    #[field(id = 2)]
    pub skeleton: AssetRef<Skeleton>,
}

impl AnimRig {
    /// A rig from its two assets.
    #[must_use]
    pub const fn new(machine: AssetRef<StateMachine>, skeleton: AssetRef<Skeleton>) -> Self {
        Self { machine, skeleton }
    }
}

/// Where a character's state machine is: the state, its phase before and after this tick's
/// advance, the outgoing state and its phase while a transition blends, how long the state
/// has run, and whether it was entered or finished this tick. Written by `anim::advance`
/// alone, every tick, in `Stage::Simulation`; read by the game after it ([`phase_crossed`]
/// is section 6.1's `footsteps` test), by `anim::evaluate`, by extract (presentation
/// evaluates the render pose from it) and by replication (the animation phase is persistent
/// state, architecture section 15.1). Every field is an integer, so a client's prediction
/// of it compares with the server's by bytes and never needs a tolerance.
///
/// Spawn a character with [`UNSET`](Self::UNSET): the first advance enters the machine's
/// initial state. The fields are private so that the invariants below hold wherever a game
/// reads them: `previous_phase` is the phase the current state had before this tick's
/// advance (its entry phase on the tick it was entered), `ticks_in_state` counts the ticks
/// since entry including this one, and the outgoing state equals the state when no blend
/// is in progress.
///
/// [`phase_crossed`]: Self::phase_crossed
#[component(key = "anim::state", persistent, replicated, extracted)]
#[derive(PartialEq, Eq, Debug)]
pub struct AnimState {
    /// The phase after this tick's advance.
    #[field(id = 1)]
    phase: Phase,
    /// The phase before this tick's advance, in the current state.
    #[field(id = 2)]
    previous_phase: Phase,
    /// The outgoing state's phase while a transition blends; unused otherwise.
    #[field(id = 3)]
    blend_phase: Phase,
    /// Ticks since the state was entered, this tick included; one on the entry tick.
    #[field(id = 4)]
    ticks_in_state: u32,
    /// The state, or `u16::MAX` before the first advance.
    #[field(id = 5)]
    state: u16,
    /// The outgoing state while a transition blends; equal to `state` otherwise.
    #[field(id = 6)]
    previous_state: u16,
    /// The length of the blend in ticks, from the transition; zero when none is in progress.
    #[field(id = 7)]
    blend_ticks: u16,
    /// [`Self::ENTERED`] and [`Self::FINISHED`].
    #[field(id = 8)]
    flags: u8,
    #[field(padding)]
    _pad: Padding<1>,
}

impl AnimState {
    /// The state was entered this tick.
    const ENTERED: u8 = 1;
    /// The state's one-shot clip has reached [`Phase::END`], this tick or earlier.
    const FINISHED: u8 = 1 << 1;

    /// No state yet: what a character is spawned with. The first `anim::advance` enters the
    /// machine's initial state at phase zero and marks it entered.
    pub const UNSET: Self = Self {
        phase: Phase::ZERO,
        previous_phase: Phase::ZERO,
        blend_phase: Phase::ZERO,
        ticks_in_state: 0,
        state: u16::MAX,
        previous_state: u16::MAX,
        blend_ticks: 0,
        flags: 0,
        _pad: pad(),
    };

    /// `state` entered at phase zero, as the advance that enters it leaves it before the
    /// phase moves: for tests and for a game that spawns a character mid-machine.
    #[must_use]
    pub const fn entering(state: StateIndex) -> Self {
        Self {
            phase: Phase::ZERO,
            previous_phase: Phase::ZERO,
            blend_phase: Phase::ZERO,
            ticks_in_state: 1,
            state,
            previous_state: state,
            blend_ticks: 0,
            flags: Self::ENTERED,
            _pad: pad(),
        }
    }

    /// The current state, or `None` before the first advance.
    #[must_use]
    pub const fn state(&self) -> Option<StateIndex> {
        if self.state == u16::MAX { None } else { Some(self.state) }
    }

    /// Whether the current state is `state`.
    #[must_use]
    pub const fn in_state(&self, state: StateIndex) -> bool {
        self.state == state && state != u16::MAX
    }

    /// The phase after this tick's advance.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// The phase before this tick's advance: the state's entry phase on the tick it was
    /// entered, the previous tick's [`phase`](Self::phase) otherwise.
    #[must_use]
    pub const fn previous_phase(&self) -> Phase {
        self.previous_phase
    }

    /// Whether the state was entered this tick, by the first advance or by a transition.
    #[must_use]
    pub const fn entered(&self) -> bool {
        self.flags & Self::ENTERED != 0
    }

    /// Whether the state's clip is a one-shot whose phase has reached [`Phase::END`]: set on
    /// the tick it gets there and kept until the state changes. Never set in a looping
    /// state.
    #[must_use]
    pub const fn finished(&self) -> bool {
        self.flags & Self::FINISHED != 0
    }

    /// Ticks since the state was entered, this tick included; one on the entry tick.
    #[must_use]
    pub const fn ticks_in_state(&self) -> u32 {
        self.ticks_in_state
    }

    /// Whether this tick's advance passed `mark` in the current state's clip: section 6.1's
    /// `phase_crossed`. True exactly once per pass over the mark: on the tick the phase
    /// reaches or passes it, through the wrap of a looping clip, and on the entry tick for a
    /// mark at or after the entry phase up to the phase reached (so a mark at phase zero
    /// fires when a state is entered at zero). On a tick with no advance (speed zero, a
    /// missing asset) nothing is passed.
    #[must_use]
    pub const fn phase_crossed(&self, mark: Phase) -> bool {
        todo!()
    }

    /// The transition blend in progress, if any: the outgoing state, its phase, and the
    /// weight the outgoing pose keeps, `1 - ticks_in_state / blend_ticks` as one
    /// `lactose_math` division. `None` once `ticks_in_state` reaches the blend's length, and
    /// for a transition with `blend_ticks` zero.
    #[must_use]
    pub fn blend(&self) -> Option<Blend> {
        if self.blend_ticks == 0 || self.ticks_in_state >= u32::from(self.blend_ticks) {
            return None;
        }
        let weight = Scalar::ONE
            - Scalar::from_u32(self.ticks_in_state) / Scalar::from_u32(u32::from(self.blend_ticks));
        Some(Blend { from: self.previous_state, phase: self.blend_phase, weight })
    }
}

/// A transition blend in progress: what [`AnimState::blend`] reports and what
/// [`SimPose::evaluate`] takes as its outgoing pose.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Blend {
    /// The outgoing state.
    pub from: StateIndex,
    /// The outgoing state's phase this tick; it keeps advancing at its own rate.
    pub phase: Phase,
    /// The weight of the outgoing pose, from one at entry toward zero.
    pub weight: Scalar,
}

/// What the game tells the state machine: parameter values the transitions compare
/// (speed, blend inputs), level flags that hold until the game clears them, and triggers
/// that `anim::advance` consumes after reading them. Written by the game in
/// [`sets::DRIVE`](crate::sets::DRIVE) through `Mut<AnimParams>`, ideally with
/// `set_if_neq` so an unchanged tick stamps nothing; `anim::advance` stamps it only when it
/// clears a trigger. Persistent and replicated: a parameter holds across ticks like an
/// animator's parameter, so a game writes a value when it changes rather than every tick,
/// and a predicted character's parameters are corrected with its state.
#[component(key = "anim::params", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct AnimParams {
    /// The values, by [`ParamIndex`].
    #[field(id = 1)]
    pub values: PlainArray<Scalar, PARAM_SLOTS>,
    /// Level flags, one bit each, kept until the game clears them.
    #[field(id = 2)]
    pub flags: u32,
    /// Triggers, one bit each: set by the game, read by the next `anim::advance` for its
    /// transitions and cleared by it, so a trigger fires at most one transition.
    #[field(id = 3)]
    pub triggers: u32,
}

impl AnimParams {
    /// Every value zero, no flag, no trigger.
    pub const ZERO: Self =
        Self { values: PlainArray([Scalar::ZERO; PARAM_SLOTS]), flags: 0, triggers: 0 };

    /// Whether flag `bit` is set.
    #[must_use]
    pub const fn flag(&self, bit: u8) -> bool {
        self.flags & (1 << bit) != 0
    }

    /// Sets or clears flag `bit`.
    pub const fn set_flag(&mut self, bit: u8, on: bool) {
        if on {
            self.flags |= 1 << bit;
        } else {
            self.flags &= !(1 << bit);
        }
    }

    /// Sets trigger `bit`, for the next advance to consume.
    pub const fn trigger(&mut self, bit: u8) {
        self.triggers |= 1 << bit;
    }
}

/// The displacement the current state's clip moved its root by this tick, in the
/// character's own space (`-Z` forward, `+Y` up, architecture section 5.3): what the
/// character's movement system applies to its `Transform` or `Velocity`, this crate never
/// moving the root itself. Written by `anim::advance` every tick (zero when the clip has no
/// root track, during a hold, or when the rig's assets are missing) and zeroed at the input
/// sync point. During a transition blend the outgoing clip's displacement is blended in by
/// the blend weight.
#[component(key = "anim::root_motion", transient)]
#[derive(PartialEq, Debug)]
pub struct RootMotion {
    /// The root's displacement this tick, in the character's space.
    #[field(id = 1)]
    pub translation: Vec3,
    /// The root's turn this tick about `+Y`, in radians, positive to the left.
    #[field(id = 2)]
    pub yaw: Scalar,
}

impl RootMotion {
    /// No motion.
    pub const ZERO: Self = Self { translation: Vec3::ZERO, yaw: Scalar::ZERO };

    /// `root` moved by this motion: the translation turned into world space by the root's
    /// rotation and added, the yaw applied about the world's `+Y` before the root's
    /// rotation. What a kinematic character writes to its `Transform` (the physics sync
    /// sweeps it there) and a dynamic one divides by the tick delta into its `Velocity`.
    #[must_use]
    pub fn apply(&self, root: &Transform) -> Transform {
        Transform::new(
            root.translation + root.rotation * self.translation,
            Quat::from_rotation_y(self.yaw) * root.rotation,
        )
    }
}

/// The evaluated sim skeleton: every bone's pose relative to the character's root, in the
/// skeleton's bone order, for the state (and blend) the character is in this tick. Written
/// by `anim::evaluate` in `Stage::Simulation`, from the clips alone, so it is valid for the
/// whole tick whatever the root does; a reader composes a bone with the root `Transform` of
/// the moment through [`world`](Self::world): the stepped root in `Stage::Post`, which is
/// where hit volumes, hand and foot placement and sockets are read. Rewritten in
/// `Stage::Post` by `anim::fit_ragdoll` for a limp ragdoll, from the bone rows' stepped
/// transforms. Transient (zeroed at the input sync point, hashed, so SOUR catches a
/// layout-dependent evaluation) and extracted, so presentation reads the sim bones from the
/// snapshot beside the root it interpolates. A character without this component is never
/// evaluated.
#[component(key = "anim::sim_pose", transient, extracted)]
#[derive(PartialEq, Debug)]
pub struct SimPose {
    /// The bones' root-local poses; entries at and past `count` are zero.
    #[field(id = 1)]
    bones: PlainArray<BonePose, MAX_SIM_BONES>,
    /// How many bones the skeleton has.
    #[field(id = 2)]
    count: u8,
    #[field(padding)]
    _pad: Padding<7>,
}

impl SimPose {
    /// No bones: what a character is spawned with, and what a tick leaves when the rig's
    /// assets are missing.
    pub const EMPTY: Self =
        Self { bones: PlainArray([BonePose::IDENTITY; MAX_SIM_BONES]), count: 0, _pad: pad() };

    /// The skeleton's rest pose, each bone's rest composed down its parent chain: what a
    /// character shows before its first evaluation, and what a test compares an evaluation
    /// against.
    #[must_use]
    pub fn rest(skeleton: &Skeleton) -> Self {
        todo!()
    }

    /// The pose of `skeleton` at `phase` of `clip`, blended with `outgoing` when a transition
    /// is in progress: per bone, the clip's sample at the phase ([`Clip::sample`]), blended
    /// toward the outgoing clip's sample at its phase by the outgoing weight
    /// ([`BonePose::blend`]), then composed onto the parent's pose ([`BonePose::compose`]),
    /// parents first, which the skeleton's bone order guarantees. The one evaluation this
    /// crate performs per character per tick, and what the milestone 1 cost test times
    /// (`README.md`, "Proof for milestone 1"). Allocates nothing. A `clip` cooked for another
    /// skeleton, or with another bone count, gives [`EMPTY`](Self::EMPTY).
    #[must_use]
    pub fn evaluate(
        skeleton: &Skeleton,
        clip: &Clip,
        phase: Phase,
        outgoing: Option<Outgoing<'_>>,
    ) -> Self {
        todo!()
    }

    /// How many bones the pose holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count as usize
    }

    /// Whether the pose holds no bone.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The root-local poses of the bones, in skeleton order.
    #[must_use]
    pub fn bones(&self) -> &[BonePose] {
        &self.bones[..self.len()]
    }

    /// Bone `bone`'s root-local pose, or `None` past the skeleton's bones.
    #[must_use]
    pub fn bone(&self, bone: BoneIndex) -> Option<BonePose> {
        self.bones().get(usize::from(bone)).copied()
    }

    /// Bone `bone` placed by `root`: its simulation transform in world space.
    #[must_use]
    pub fn world(&self, root: &Transform, bone: BoneIndex) -> Option<Transform> {
        self.bone(bone).map(|pose| pose.in_world(root))
    }

    /// Socket `socket` of `skeleton` placed by `root`: the socket's offset composed onto its
    /// bone's pose, then placed, or `None` if the skeleton has no such socket or the pose
    /// lacks its bone.
    #[must_use]
    pub fn socket(
        &self,
        skeleton: &Skeleton,
        root: &Transform,
        socket: usize,
    ) -> Option<Transform> {
        let socket = skeleton.sockets.get(socket)?;
        self.bone(socket.bone).map(|bone| bone.compose(socket.offset).in_world(root))
    }

    /// Replaces bone `bone`'s root-local pose; a bone past the count is ignored. The
    /// ragdoll fit's write.
    pub(crate) fn set_bone(&mut self, bone: BoneIndex, pose: BonePose) {
        if usize::from(bone) < self.len() {
            self.bones[usize::from(bone)] = pose;
        }
    }
}

/// The outgoing side of a transition blend, for [`SimPose::evaluate`]: the clip of the state
/// being left, its phase this tick, and the weight its pose keeps.
#[derive(Clone, Copy)]
pub struct Outgoing<'a> {
    /// The outgoing state's clip.
    pub clip: &'a Clip,
    /// The outgoing state's phase this tick.
    pub phase: Phase,
    /// The weight of the outgoing pose, from one toward zero.
    pub weight: Scalar,
}

/// A character's bone rows: the physics bodies of its sim skeleton (`README.md`, "Bones and
/// ragdolls"), one row per bone that has a [`BoneBody`](crate::BoneBody), each owned by the
/// character (`OwnedBy`, so it migrates and despawns with it) and linked back by
/// [`BoneLink`]. While the character is animated the bones are kinematic and
/// `anim::drive_bones` sweeps them to the evaluated pose every tick; `limp` makes them
/// dynamic and the pose follows them instead. Holds references, so it is written through
/// `Commands::set` only ([`spawn`](Self::spawn), [`set_limp`](Self::set_limp)) and the
/// schedule refuses `Mut` on it. Replicated, since the kernels that read it run in the
/// prediction world; the bone rows themselves are replicated through their own components.
#[component(key = "anim::bones", persistent, replicated)]
#[derive(PartialEq, Eq, Debug)]
pub struct Bones {
    /// The bone rows by [`BoneIndex`]; the empty reference for a bone without a body.
    #[field(id = 1)]
    keys: PlainArray<EntityRef, MAX_SIM_BONES>,
    /// How many bones the skeleton has.
    #[field(id = 2)]
    count: u8,
    /// Whether the bodies drive the pose (a ragdoll) rather than the pose the bodies.
    #[field(id = 3)]
    limp: bool,
    #[field(padding)]
    _pad: Padding<6>,
}

impl Bones {
    /// Records, on `cmd`, the spawn of `owner`'s bone rows from `skeleton`, placed by `root`
    /// at `pose` (the character's current [`SimPose`], or its rest pose), and the write of
    /// this component onto `owner` naming them. For every bone with a [`BoneBody`]: a row
    /// with `Transform` (the bone's world pose), the body's `RigidBody` made
    /// [`kinematic`](lactose_physics_api::RigidBody::kinematic), its `Collider`, `OwnedBy`
    /// the owner and a [`BoneLink`] back to it; and for every such bone with a
    /// [`BoneJoint`](crate::BoneJoint) whose parent chain reaches a bone with a body, a
    /// `Joint` row to that body, owned by the bone row. The placeholders are resolved at
    /// apply, so the owner's row names its bones from the next sync point on. The calling
    /// kernel declares `writes = [Bones]`; the spawns are structural and need no
    /// declaration. A second call on an owner that has bones spawns a second set: despawn
    /// the owner's bones first (despawning the owner does).
    pub fn spawn(
        cmd: &mut Commands<'_>,
        owner: StableKey,
        skeleton: &Skeleton,
        root: &Transform,
        pose: &SimPose,
    ) {
        todo!()
    }

    /// Records, on `cmd`, the switch of `owner`'s bones between animated and limp: this
    /// component rewritten with `limp`, every bone row's `RigidBody` rewritten with the
    /// skeleton's body made [`dynamic`](lactose_physics_api::RigidBody::dynamic) when limp
    /// and `kinematic` otherwise (which the physics sync pushes as a change of the body), and
    /// every bone row's [`BoneLink`] rewritten with `limp`. The calling kernel declares
    /// `writes = [Bones, BoneLink, RigidBody]`. Nothing happens when `limp` equals the
    /// current value.
    pub fn set_limp(
        &self,
        cmd: &mut Commands<'_>,
        owner: StableKey,
        skeleton: &Skeleton,
        limp: bool,
    ) {
        todo!()
    }

    /// Whether the bodies drive the pose.
    #[must_use]
    pub const fn is_limp(&self) -> bool {
        self.limp
    }

    /// How many bones the skeleton has, bodies or not.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count as usize
    }

    /// Whether the skeleton has no bone.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The row of bone `bone`, or `None` for a bone without a body or past the skeleton.
    /// The stored key is the origin key.
    #[must_use]
    pub fn key(&self, bone: BoneIndex) -> Option<StableKey> {
        self.keys.get(usize::from(bone)).and_then(|key| key.get())
    }
}

/// A bone row's link to the character it belongs to: which bone of the owner's skeleton it
/// is, and whether the owner is limp (so that only a ragdoll's bones report where the step
/// left them). Written beside `OwnedBy` by [`Bones::spawn`], rewritten by
/// [`Bones::set_limp`], through commands, since it holds a reference. Replicated, like
/// [`Bones`].
#[component(key = "anim::bone_link", persistent, replicated)]
#[derive(PartialEq, Eq, Debug)]
pub struct BoneLink {
    /// The character that owns the bone, as the origin key.
    #[field(id = 1)]
    pub owner: EntityRef,
    /// Which bone of the owner's skeleton.
    #[field(id = 2)]
    pub bone: BoneIndex,
    /// Whether the owner's bodies drive its pose.
    #[field(id = 3)]
    pub limp: bool,
    #[field(padding)]
    _pad: Padding<6>,
}

impl BoneLink {
    /// A link to bone `bone` of `owner`, animated.
    #[must_use]
    pub const fn new(owner: StableKey, bone: BoneIndex) -> Self {
        Self { owner: EntityRef::new(owner), bone, limp: false, _pad: pad() }
    }
}
