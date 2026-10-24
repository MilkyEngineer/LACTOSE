//! The events this crate emits within a tick (architecture section 6.1): an authored clip
//! event the phase passed, to the character's own key; and a limp ragdoll's bone row
//! reporting where the step left it, to its owner.

use lactose_core::StableKey;
use lactose_ecs::Event;
use lactose_physics_api::Transform;

use crate::components::{BoneIndex, StateIndex};
use crate::keys::AnimEventKey;
use crate::phase::Phase;

/// An authored clip event whose phase the character's state passed this tick, emitted by
/// `anim::advance` with the character's own origin key as the target, so a later kernel on
/// the same row reads its events with `Events::<AnimEvent>::for_target(*key)` and a
/// once-per-tick system reads every character's with `iter`. Emitted in the order the
/// advance passed them: ascending by phase, and across a wrap those in `(from, END]`
/// ascending, then those in `[ZERO, to]` ascending; once per pass over the event's phase,
/// by the rule of `AnimState::phase_crossed` (`README.md`, "Phase and crossings"); the
/// phase is the event's authored phase, not the state's. Cleared at the input sync point
/// like every event, and never in a snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AnimEvent {
    /// The character whose clip carried the event: its origin key.
    pub target: StableKey,
    /// The authored event.
    pub event: AnimEventKey,
    /// The state whose clip carried it.
    pub state: StateIndex,
    /// The event's authored phase in that clip.
    pub phase: Phase,
}

impl Event for AnimEvent {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// Where the step left a limp ragdoll's bone, emitted by `anim::report_bones` in
/// `Stage::Post` from the bone row's stepped `Transform`, to the character that owns it
/// ([`BoneLink::owner`](crate::BoneLink::owner)); `anim::fit_ragdoll` reads them to rebuild
/// the owner's [`SimPose`](crate::SimPose) (`README.md`, "Bones and ragdolls"). Only a bone
/// row whose link says `limp` emits, so an animated character costs no event.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BoneMoved {
    /// The character that owns the bone: its origin key.
    pub owner: StableKey,
    /// Which bone of the owner's skeleton.
    pub bone: BoneIndex,
    /// The bone row's stepped transform, in world space.
    pub transform: Transform,
}

impl Event for BoneMoved {
    fn target(&self) -> StableKey {
        self.owner
    }
}
