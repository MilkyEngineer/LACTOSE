//! Contact and trigger events (architecture section 6.1): what the step emits for the
//! readers of `Stage::Post`, indexed by the entity they are for.

use lactose_core::StableKey;
use lactose_ecs::Event;
use lactose_math::{Scalar, Vec3};

/// Whether a pair started or stopped touching this tick. Pairs that go on touching emit
/// nothing: a resting contact is found with a query.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ContactPhase {
    /// The pair began touching during this tick's step.
    Started,
    /// The pair stopped touching during this tick's step.
    Stopped,
}

/// Two solid colliders started or stopped touching. Emitted twice per pair, once for each
/// body as `target`, so `Events<ContactEvent>::for_target(key)` finds every contact of
/// `key`'s entity. The step emits a tick's events in ascending (`target`, `other`, `phase`)
/// order, whatever order the backend reported them in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ContactEvent {
    /// The entity the event is for.
    pub target: StableKey,
    /// The entity it touched.
    pub other: StableKey,
    /// Started or stopped.
    pub phase: ContactPhase,
    /// A contact point in world space; zero when stopped.
    pub point: Vec3,
    /// The contact normal, pointing out of `other` towards `target`; zero when stopped.
    pub normal: Vec3,
    /// The total impulse the solver applied along the normal during the step, in mass units
    /// times units per second; zero when stopped.
    pub impulse: Scalar,
}

impl Event for ContactEvent {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// A sensor collider and another collider started or stopped overlapping. Emitted twice per
/// pair, once for each entity as `target`, in ascending (`target`, `other`, `phase`) order
/// after the tick's contact events.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TriggerEvent {
    /// The entity the event is for.
    pub target: StableKey,
    /// The entity it overlapped.
    pub other: StableKey,
    /// Started or stopped.
    pub phase: ContactPhase,
}

impl Event for TriggerEvent {
    fn target(&self) -> StableKey {
        self.target
    }
}
