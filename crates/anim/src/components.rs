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
use lactose_ecs::{Commands, OwnedBy};
use lactose_math::{Quat, Scalar, Vec3};
use lactose_physics_api::{BodyType, Joint, Transform, Velocity};
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

    /// Every scalar zero: not a valid pose (its rotation has no length), the bytes a zeroed
    /// transient column holds and what an unused [`SimPose`] slot is.
    pub const ZERO: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::new(Scalar::ZERO, Scalar::ZERO, Scalar::ZERO, Scalar::ZERO),
    };

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
    pub(crate) phase: Phase,
    /// The phase before this tick's advance, in the current state.
    #[field(id = 2)]
    pub(crate) previous_phase: Phase,
    /// The outgoing state's phase while a transition blends; unused otherwise.
    #[field(id = 3)]
    pub(crate) blend_phase: Phase,
    /// Ticks since the state was entered, this tick included; one on the entry tick, zero
    /// while the state's clip has been missing since its entry (a hold).
    #[field(id = 4)]
    pub(crate) ticks_in_state: u32,
    /// The state, or `u16::MAX` before the first advance.
    #[field(id = 5)]
    pub(crate) state: u16,
    /// The outgoing state while a transition blends; equal to `state` otherwise.
    #[field(id = 6)]
    pub(crate) previous_state: u16,
    /// The length of the blend in ticks, from the transition; zero when none is in progress.
    #[field(id = 7)]
    pub(crate) blend_ticks: u16,
    /// [`Self::ENTERED`], [`Self::FINISHED`], [`Self::ENTRY_SET`] and [`Self::HELD`].
    #[field(id = 8)]
    pub(crate) flags: u8,
    #[field(padding)]
    _pad: Padding<1>,
}

impl AnimState {
    /// The state was entered this tick.
    pub(crate) const ENTERED: u8 = 1;
    /// The state's one-shot clip has reached [`Phase::END`], this tick or earlier.
    pub(crate) const FINISHED: u8 = 1 << 1;
    /// This tick's entry set the phase (the initial state, or a transition without
    /// `keep_phase`) rather than keeping the outgoing state's, so a mark at the entry phase
    /// counts on the entry tick; a kept phase was already passed by the tick that reached
    /// it.
    pub(crate) const ENTRY_SET: u8 = 1 << 2;
    /// This tick's advance was a hold (the machine or a clip missing): set by the hold,
    /// cleared by an entry (of the initial state, or by a transition) and by the next
    /// advance that moves the phase. The rest of the row is as the last advance left it, so
    /// the accessors that report what happened this tick read false while it is set rather
    /// than repeat that advance. A hold on the tick of an entry (the initial state's, or a
    /// transition's whose target clip is missing) does not set it: that tick's row is the
    /// entry, which is reported then and on no other tick.
    pub(crate) const HELD: u8 = 1 << 3;

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
            flags: Self::ENTERED | Self::ENTRY_SET,
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
    /// False on a held tick, which reports nothing (`README.md`, "The state machine"). The
    /// tick of an entry is not held even when the entered state's clip was missing, so the
    /// entry is reported on it.
    #[must_use]
    pub const fn entered(&self) -> bool {
        self.flags & (Self::ENTERED | Self::HELD) == Self::ENTERED
    }

    /// Whether this tick's entry set the phase (the initial state, or a transition without
    /// `keep_phase`) rather than keeping the outgoing state's phase. False when the state
    /// was not entered this tick, and on a held tick.
    #[must_use]
    pub const fn entry_was_set(&self) -> bool {
        let mask = Self::ENTERED | Self::ENTRY_SET | Self::HELD;
        self.flags & mask == (Self::ENTERED | Self::ENTRY_SET)
    }

    /// Whether the state's clip is a one-shot whose phase has reached [`Phase::END`]: set on
    /// the tick it gets there and kept until the state changes. Never set in a looping
    /// state.
    #[must_use]
    pub const fn finished(&self) -> bool {
        self.flags & Self::FINISHED != 0
    }

    /// Ticks since the state was entered, this tick included; one on the entry tick, zero
    /// while the state's clip has been missing since its entry (a hold).
    #[must_use]
    pub const fn ticks_in_state(&self) -> u32 {
        self.ticks_in_state
    }

    /// Whether this tick's advance passed `mark` in the current state's clip: section 6.1's
    /// `phase_crossed`, [`Phase::passed`] from the previous phase to the phase, plus the
    /// entry phase itself on the tick the state was entered with a set phase (the initial
    /// state, or a transition without `keep_phase`), so a mark at phase zero fires when a
    /// state is entered at zero. A kept phase was already passed by the tick that reached
    /// it, so the entry tick of a `keep_phase` transition reports only what its own advance
    /// passed. True exactly once per pass over the mark: on the tick the phase reaches or
    /// passes it, through the wrap of a looping clip. A tick with no advance passes
    /// nothing: at rate zero the two phases are equal, and on a held tick (the machine or a
    /// clip missing), whose row keeps the last advance's phases, this is false for every
    /// mark. The tick of an entry is not held even when the entered state's clip was
    /// missing, so a set entry phase is reported on it, once.
    #[must_use]
    pub const fn phase_crossed(&self, mark: Phase) -> bool {
        if self.flags & Self::HELD != 0 {
            return false;
        }
        Phase::passed(self.previous_phase, self.phase, mark)
            || (self.entry_was_set() && mark.0 == self.previous_phase.0)
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
///
/// D124's rule applies to the values: a replicated field compares by canonical bits, so a
/// value derived from a solver-written row (a speed from `Velocity`) is quantised to an
/// integer grid first ([`quantise`](Self::quantise)) or taken from the input sample, or
/// every residual the root's tolerance absorbs becomes a mismatch here and a correction.
/// The bits are checked: a `bit` of 32 or more is a debug assertion, and in a release
/// build the flag reads false and the write does nothing.
#[component(key = "anim::params", persistent, replicated)]
#[derive(PartialEq, Debug)]
pub struct AnimParams {
    /// The values, by [`ParamIndex`].
    #[field(id = 1)]
    pub values: PlainArray<Scalar, PARAM_SLOTS>,
    /// Level flags, one bit each, kept until the game clears them.
    #[field(id = 2)]
    pub flags: u32,
    /// Triggers, one bit each: set by the game, read by the next advance that tries
    /// transitions and cleared by it, so a trigger fires at most one transition
    /// (`README.md`, "The state machine", says which advances try).
    #[field(id = 3)]
    pub triggers: u32,
}

impl AnimParams {
    /// Every value zero, no flag, no trigger.
    pub const ZERO: Self =
        Self { values: PlainArray([Scalar::ZERO; PARAM_SLOTS]), flags: 0, triggers: 0 };

    /// The mask of `bit`, or zero for a bit past the word (a debug assertion).
    const fn mask(bit: u8) -> u32 {
        debug_assert!(bit < 32, "a flag or trigger bit is below 32");
        match 1u32.checked_shl(bit as u32) {
            Some(mask) => mask,
            None => 0,
        }
    }

    /// Whether flag `bit` is set.
    #[must_use]
    pub const fn flag(&self, bit: u8) -> bool {
        self.flags & Self::mask(bit) != 0
    }

    /// Sets or clears flag `bit`.
    pub const fn set_flag(&mut self, bit: u8, on: bool) {
        if on {
            self.flags |= Self::mask(bit);
        } else {
            self.flags &= !Self::mask(bit);
        }
    }

    /// Sets trigger `bit`, for the next advance that tries transitions to consume.
    pub const fn trigger(&mut self, bit: u8) {
        self.triggers |= Self::mask(bit);
    }

    /// `value` snapped to the nearest multiple of `1 / steps_per_unit`: the one-liner a game
    /// uses before it stores a solver-derived quantity (a speed from `Velocity`) in a value
    /// (D124). Two quantities a residual apart quantise to the same bits unless a grid
    /// midpoint, an odd multiple of `1 / (2 * steps_per_unit)`, lies between them; such a
    /// straddle is rare for a residual far below a step and is an ordinary correction when it
    /// happens, not a mismatch every tick. `round` toward the nearest integer through
    /// `lactose_math`, then one division; a non-finite `value` or a zero `steps_per_unit`
    /// gives zero.
    #[must_use]
    pub fn quantise(value: Scalar, steps_per_unit: u32) -> Scalar {
        if steps_per_unit == 0 || !value.is_finite() {
            return Scalar::ZERO;
        }
        let steps = Scalar::from_u32(steps_per_unit);
        Scalar::from_i64((value * steps).round().to_i64()) / steps
    }
}

/// The displacement the current state's clip moved its root by this tick, in the
/// character's own space as it was at the start of the tick (`-Z` forward, `+Y` up,
/// architecture section 5.3; `Clip::root_motion` turns the track's clip-start-space
/// difference back by the yaw already turned): what the character's movement system
/// applies to its `Transform` or `Velocity`, this crate never moving the root itself.
/// Written by `anim::advance` every tick (zero when the clip has no root track, during a
/// hold, or when the rig's assets are missing) and zeroed at the input sync point. During
/// a transition blend the outgoing clip's displacement is blended in by the blend weight.
#[component(key = "anim::root_motion", transient)]
#[derive(PartialEq, Debug)]
pub struct RootMotion {
    /// The root's displacement this tick, in the character's space at the tick's start.
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
    /// rotation. What a kinematic character writes to its `Transform`: the physics sync
    /// sweeps it there during the step.
    #[must_use]
    pub fn apply(&self, root: &Transform) -> Transform {
        Transform::new(
            root.translation + root.rotation * self.translation,
            Quat::from_rotation_y(self.yaw) * root.rotation,
        )
    }

    /// `current`, the dynamic root's velocity, with the horizontal motion and the turn this
    /// motion covers in one tick of `dt` seconds: the translation turned into world space
    /// by the root's rotation, its `x` and `z` divided by `dt` as `linear.x` and `linear.z`,
    /// and the yaw divided by `dt` as `angular.y`, about the world's `+Y`. Every other
    /// component is `current`'s: the vertical is the game's (gravity, which the solver has
    /// accumulated in `linear.y`, and jumps), and the clip's own vertical motion is not
    /// applied to a dynamic root. What a dynamic character writes to its `Velocity` (the
    /// physics sync sets it, the step integrates it): the solver then turns the body as it
    /// moves it, with no placement and so no `Teleported`, which keeps clients
    /// interpolating; such a character locks `ROTATION_X | ROTATION_Z` and leaves
    /// `ROTATION_Y` free, and takes the solver's own contribution to its yaw (contact
    /// torque, damping) as the price of being dynamic. A zero `dt` gives `current` with
    /// the horizontal linear and the yaw rate zero.
    #[must_use]
    pub fn velocity(&self, root: &Transform, current: &Velocity, dt: Scalar) -> Velocity {
        let (linear, yaw_rate) = if dt == Scalar::ZERO {
            (Vec3::ZERO, Scalar::ZERO)
        } else {
            (root.rotation * self.translation / dt, self.yaw / dt)
        };
        Velocity {
            linear: Vec3::new(linear.x, current.linear.y, linear.z),
            angular: Vec3::new(current.angular.x, yaw_rate, current.angular.z),
        }
    }
}

/// The evaluated sim skeleton: every bone's pose relative to the character's root, in the
/// skeleton's bone order, for the state (and blend) the character is in this tick. For an
/// animated character it is written by `anim::evaluate` in `Stage::Simulation`, from the
/// clips alone, so it is valid for the whole tick whatever the root does; a reader composes
/// a bone with the root `Transform` of the moment through [`world`](Self::world): the
/// stepped root in `Stage::Post`, which is where hit volumes, hand and foot placement and
/// sockets are read. For a limp ragdoll `anim::evaluate` leaves it [`EMPTY`](Self::EMPTY)
/// and, in the server world, `anim::fit_ragdoll` writes it in `Stage::Post` from the bone
/// rows' stepped transforms, so a limp character's pose exists from
/// [`sets::FIT`](crate::sets::FIT) on and a `Stage::Simulation` reader finds it empty; in
/// the prediction world, which has no bone rows, it stays empty all tick. Transient
/// (zeroed at the input sync point, hashed, so SOUR catches a layout-dependent evaluation)
/// and extracted, so presentation reads the sim bones from the snapshot beside the root it
/// interpolates. A character without this component is never evaluated.
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
    /// No bones, every slot zero: the bytes of a zeroed transient column, what a character
    /// is spawned with, and what a tick leaves when the rig's assets are missing or the
    /// character is limp before the fit. Writing it onto a zeroed row changes no byte.
    pub const EMPTY: Self =
        Self { bones: PlainArray([BonePose::ZERO; MAX_SIM_BONES]), count: 0, _pad: pad() };

    /// The skeleton's rest pose, each bone's rest composed down its parent chain: what a
    /// character shows before its first evaluation, and what a test compares an evaluation
    /// against. [`EMPTY`](Self::EMPTY) for a skeleton of no bone, of more than
    /// [`MAX_SIM_BONES`], or whose parents do not precede their children.
    #[must_use]
    pub fn rest(skeleton: &Skeleton) -> Self {
        Self::compose(skeleton, |bone| skeleton.bones[usize::from(bone)].rest)
    }

    /// Every bone of `skeleton` from its parent-local pose `local`, composed onto its
    /// parent's pose in bone order; [`EMPTY`](Self::EMPTY) for a skeleton of no bone or
    /// more than [`MAX_SIM_BONES`], or one whose parents do not precede their children
    /// (one `Skeleton::validate` refuses). A bone without a parent is root-local as it
    /// stands.
    fn compose(skeleton: &Skeleton, mut local: impl FnMut(BoneIndex) -> BonePose) -> Self {
        let count = skeleton.bones.len();
        if count == 0 || count > MAX_SIM_BONES {
            return Self::EMPTY;
        }
        let mut pose = Self::EMPTY;
        for (index, bone) in skeleton.bones.iter().enumerate() {
            // Below `MAX_SIM_BONES`, so the index is a `BoneIndex`.
            let local = local(index as BoneIndex);
            pose.bones[index] = match bone.parent {
                None => local,
                Some(parent) if usize::from(parent) < index => {
                    pose.bones[usize::from(parent)].compose(local)
                }
                Some(_) => return Self::EMPTY,
            };
        }
        pose.count = count as u8;
        pose
    }

    /// The pose of `skeleton` at `phase` of `clip`, blended with `outgoing` when a transition
    /// is in progress: per bone, the clip's sample at the phase ([`Clip::sample`]), blended
    /// toward the outgoing clip's sample at its phase by the outgoing weight
    /// ([`BonePose::blend`]), then composed onto the parent's pose ([`BonePose::compose`]),
    /// parents first, which the skeleton's bone order guarantees. The one evaluation this
    /// crate performs per character per tick, and what the milestone 1 cost test times
    /// (`README.md`, "Proof for milestone 1"). Allocates nothing. A `clip` (or an outgoing
    /// clip) with another bone count than the skeleton's gives [`EMPTY`](Self::EMPTY), as
    /// does a skeleton `Skeleton::validate` refuses. Which skeleton a clip was cooked for is
    /// an asset ID this function does not see: `anim::evaluate` compares
    /// [`Clip::skeleton`] with the rig's and writes `EMPTY` for another skeleton's clip.
    #[must_use]
    pub fn evaluate(
        skeleton: &Skeleton,
        clip: &Clip,
        phase: Phase,
        outgoing: Option<Outgoing<'_>>,
    ) -> Self {
        let bones = skeleton.bones.len();
        if usize::from(clip.bones) != bones
            || outgoing.is_some_and(|outgoing| usize::from(outgoing.clip.bones) != bones)
        {
            return Self::EMPTY;
        }
        // One cursor per clip: the frames and the fraction are the same for every bone.
        let cursor = clip.cursor(phase);
        let outgoing = outgoing.map(|outgoing| (outgoing, outgoing.clip.cursor(outgoing.phase)));
        Self::compose(skeleton, |bone| {
            let pose = clip.pose_at(cursor, bone);
            match outgoing {
                None => pose,
                Some((outgoing, at)) => {
                    pose.blend(outgoing.clip.pose_at(at, bone), outgoing.weight)
                }
            }
        })
    }

    /// The pose of a limp character: each bone with a world `reported` by its bone row
    /// taken into `root`'s space, each other bone `local(bone)` composed onto its parent's
    /// fitted pose. [`EMPTY`](Self::EMPTY) as [`compose`](Self::compose) gives it.
    pub(crate) fn fit(
        skeleton: &Skeleton,
        root: &Transform,
        reported: &[Option<Transform>; MAX_SIM_BONES],
        mut local: impl FnMut(BoneIndex) -> BonePose,
    ) -> Self {
        let mut pose = Self::compose(skeleton, |_| BonePose::IDENTITY);
        if pose.is_empty() {
            return pose;
        }
        let to_root = root.rotation.conjugate();
        for (index, bone) in skeleton.bones.iter().enumerate() {
            // Below `MAX_SIM_BONES`, so the index is a `BoneIndex`.
            let index = index as BoneIndex;
            let fitted = match reported[usize::from(index)] {
                Some(world) => BonePose::new(
                    to_root * (world.translation - root.translation),
                    to_root * world.rotation,
                ),
                None => {
                    let local = local(index);
                    match bone.parent.and_then(|parent| pose.bone(parent)) {
                        Some(parent) => parent.compose(local),
                        None => local,
                    }
                }
            };
            pose.set_bone(index, fitted);
        }
        pose
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
/// schedule refuses `Mut` on it. Replicated, since `anim::evaluate`, which reads it, runs
/// in the prediction world; its value is the server's alone.
///
/// Bone rows exist in the server world alone: [`spawn`](Self::spawn) and
/// [`set_limp`](Self::set_limp) are called from server-only systems and the bone systems
/// declare `worlds = [server]`, so the prediction world has none and its queries hit
/// capsules only (`README.md`, "Prediction and replication"). Do not use this component in
/// a networked session until `lactose-net-api`'s `Unreplicated` marker lands: until then a
/// bone row's `Transform`, `RigidBody`, `Collider` and [`BoneLink`] replicate like any
/// row's.
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
    /// with `Transform` (the bone's world pose, its rotation normalised), the body's
    /// `RigidBody` made [`kinematic`](lactose_physics_api::RigidBody::kinematic), its
    /// `Collider`, `OwnedBy` the owner and a [`BoneLink`] back to it (the physics
    /// write-back adds a `Velocity` at the first step that sweeps it); and for every such
    /// bone with a [`BoneJoint`](crate::BoneJoint) whose parent chain reaches a bone with
    /// a body, a `Joint` row to that body, owned by the bone row. Called only from a system that
    /// declares `worlds = [server]`: the spawns are then not predicted, each row's origin
    /// key is a runtime `NetId` and no claim passes on, so `lactose-predict`'s destruction
    /// of an unechoed predicted spawn never applies to them, and the prediction world has
    /// no bone rows (`README.md`, "Prediction and replication"). Do not use it in a
    /// networked session until `lactose-net-api`'s `Unreplicated` marker lands, which this
    /// function will put on every row it spawns; until then the rows replicate like any
    /// row. The placeholders are resolved at apply, so the
    /// owner's row names its bones from the next sync point on. The calling kernel declares
    /// `writes = [Bones]`; the spawns are structural and need no declaration. Call it at
    /// most once per owner per tick: a call on an owner whose `existing` bones are `Some`
    /// does nothing and is logged at target `anim`, but `existing` is the row as of the
    /// last sync point, which an earlier call in the same tick has not written yet, so a
    /// second call in that tick records a second set of rows that `Bones` never names and
    /// nothing sweeps.
    pub fn spawn(
        cmd: &mut Commands<'_>,
        owner: StableKey,
        existing: Option<&Self>,
        skeleton: &Skeleton,
        root: &Transform,
        pose: &SimPose,
    ) {
        if existing.is_some() {
            tracing::warn!(
                target: "anim",
                owner = ?owner,
                "Bones::spawn on a character that already has bones: nothing spawned"
            );
            return;
        }
        let count = skeleton.bones.len();
        if count > MAX_SIM_BONES {
            tracing::warn!(
                target: "anim",
                owner = ?owner,
                bones = count,
                "Bones::spawn with a skeleton of more than MAX_SIM_BONES bones: nothing spawned"
            );
            return;
        }
        let rest;
        let pose = if pose.len() == count {
            pose
        } else {
            rest = SimPose::rest(skeleton);
            &rest
        };
        let owned = OwnedBy { owner: EntityRef::new(owner) };
        let mut keys = [EntityRef::NONE; MAX_SIM_BONES];
        for (index, bone) in skeleton.bones.iter().enumerate() {
            let Some(body) = &bone.body else { continue };
            // Below `MAX_SIM_BONES`, so the index is a `BoneIndex`.
            let transform = row_transform(pose.bones[index].in_world(root));
            let mut rigid = body.body;
            rigid.body_type = BodyType::Kinematic;
            let link = BoneLink::new(owner, index as BoneIndex);
            let row = cmd.spawn((transform, rigid, body.collider, owned, link));
            keys[index] = EntityRef::new(row);
            let Some(joint) = &body.joint else { continue };
            // Parents precede their children, so the ancestor's row is already spawned.
            let Some(above) = skeleton.body_above(index).and_then(|above| keys[above].get()) else {
                continue;
            };
            let mut row_joint = Joint::new(joint.kind, above, row)
                .with_anchors(joint.anchor_parent, joint.anchor_bone)
                .with_axis(joint.axis);
            if let Some((min, max)) = joint.limits {
                row_joint = row_joint.with_limits(min, max);
            }
            cmd.spawn((row_joint, OwnedBy { owner: EntityRef::new(row) }));
        }
        let bones = Self { keys: PlainArray(keys), count: count as u8, limp: false, _pad: pad() };
        cmd.set(owner, bones);
    }

    /// Records, on `cmd`, the switch of `owner`'s bones between animated and limp: this
    /// component rewritten with `limp`, every bone row's `RigidBody` rewritten with the
    /// skeleton's body made [`dynamic`](lactose_physics_api::RigidBody::dynamic) when limp
    /// and `kinematic` otherwise (which the physics sync pushes as a change of the body), and
    /// every bone row's [`BoneLink`] rewritten with `limp`. Called only from a system that
    /// declares `worlds = [server]`, as [`spawn`](Self::spawn) is; the calling kernel
    /// declares `writes = [Bones, BoneLink, RigidBody]`. Nothing happens when `limp` equals
    /// the current value. The character's root is not moved: a game that moves a local
    /// player's root while it is limp, with its claim kept, makes every such frame a
    /// correction (`README.md`, "Bones and ragdolls", says what to do instead).
    pub fn set_limp(
        &self,
        cmd: &mut Commands<'_>,
        owner: StableKey,
        skeleton: &Skeleton,
        limp: bool,
    ) {
        if self.limp == limp {
            return;
        }
        cmd.set(owner, Self { limp, ..*self });
        let body_type = if limp { BodyType::Dynamic } else { BodyType::Kinematic };
        for (index, key) in self.keys.iter().enumerate().take(self.len()) {
            let Some(row) = key.get() else { continue };
            if let Some(body) = skeleton.bones.get(index).and_then(|bone| bone.body.as_ref()) {
                let mut rigid = body.body;
                rigid.body_type = body_type;
                cmd.set(row, rigid);
            }
            let link = BoneLink { limp, ..BoneLink::new(owner, index as BoneIndex) };
            cmd.set(row, link);
        }
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

/// `transform` with its rotation normalised: what [`Bones::spawn`] and `anim::drive_bones`
/// write to a bone row. `Skeleton::validate` and `Clip::validate` hold each rotation to the
/// backends' unit tolerance on its own, but a row's rotation is their product down the
/// chain and the root's, whose error grows with depth past that tolerance, and the backend
/// would refuse the row at every sync. `Quat::normalize` is a correctly rounded square root
/// and four divisions, IEEE basic operations through `lactose_math` with no approximate
/// reciprocal square root, which architecture section 8.2 permits. A rotation without
/// a positive finite length (a root that is no rotation) is left as it is rather than made
/// NaN; the backend refuses it either way.
pub(crate) fn row_transform(transform: Transform) -> Transform {
    let length = transform.rotation.length();
    if length > Scalar::ZERO && length.is_finite() {
        Transform::new(transform.translation, transform.rotation.normalize())
    } else {
        transform
    }
}

/// A bone row's link to the character it belongs to: which bone of the owner's skeleton it
/// is, and whether the owner is limp (so that only a ragdoll's bones report where the step
/// left them). Written beside `OwnedBy` by [`Bones::spawn`], rewritten by
/// [`Bones::set_limp`], through commands, since it holds a reference. Replicated, like
/// [`Bones`]; it sits on bone rows alone, which exist in the server world alone.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The row a hold leaves: `state` as the last advance left it, marked held.
    fn held(state: AnimState) -> AnimState {
        AnimState { flags: state.flags | AnimState::HELD, ..state }
    }

    #[test]
    fn an_entry_then_a_hold_is_reported_once() {
        // The initial state entered at phase zero (step 2), then its clip missing (step 4):
        // the hold does not mark that tick held, so the row is the entry before step 6, and
        // the entry and its entry phase are reported on it.
        let entry_tick = AnimState { ticks_in_state: 0, ..AnimState::entering(0) };
        assert!(entry_tick.entered() && entry_tick.entry_was_set());
        assert!(entry_tick.phase_crossed(Phase::ZERO));
        assert!(!entry_tick.phase_crossed(Phase::HALF));

        // Every later tick the clip stays missing marks it held and reports nothing.
        let held = held(entry_tick);
        for mark in [Phase::ZERO, Phase::HALF, Phase::END] {
            assert!(!held.phase_crossed(mark), "{mark:?}");
        }
        assert!(!held.entered());
        assert!(!held.entry_was_set());
        assert_eq!(held.state(), Some(0));
        assert_eq!(held.ticks_in_state(), 0);

        // The first advance once the clip arrives (no transition, entered cleared, the
        // held mark cleared) passes `(ZERO, phase]`, so phase zero is not reported again.
        let arrived = AnimState {
            phase: Phase::from_fraction(1, 60),
            ticks_in_state: 1,
            flags: 0,
            ..entry_tick
        };
        assert!(!arrived.entered());
        assert!(!arrived.phase_crossed(Phase::ZERO));
        assert!(arrived.phase_crossed(Phase::from_fraction(1, 60)));
    }

    #[test]
    fn a_hold_after_an_advance_does_not_repeat_its_crossings() {
        // An advance from 0.25 to 0.75 passed HALF; the holds after it keep both phases.
        let advanced = AnimState {
            phase: Phase::from_fraction(3, 4),
            previous_phase: Phase::from_fraction(1, 4),
            ticks_in_state: 7,
            flags: 0,
            ..AnimState::entering(2)
        };
        assert!(advanced.phase_crossed(Phase::HALF));
        assert!(!held(advanced).phase_crossed(Phase::HALF));
        assert_eq!(held(advanced).phase(), advanced.phase());

        // A transition taken on the tick before the hold is not entered again on it.
        let transition = AnimState { flags: AnimState::ENTERED, ..advanced };
        assert!(transition.entered() && !transition.entry_was_set());
        assert!(!held(transition).entered());
        assert!(!held(transition).phase_crossed(Phase::HALF));
    }
}
