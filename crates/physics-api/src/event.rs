//! Contact and trigger events (architecture section 6.1): what the step emits for the
//! readers of `Stage::Post`, indexed by the entity they are for.

use lactose_core::StableKey;
use lactose_ecs::Event;
use lactose_math::{Scalar, Vec3};

/// What a contact event reports about a pair. Pairs that go on touching below the impact
/// threshold emit nothing: a resting contact is found with a query.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ContactPhase {
    /// The pair began touching during this tick's step. The solver may report it for a
    /// speculative contact a little before the bodies meet, with an impulse near zero: a
    /// damage rule reads [`Impact`](Self::Impact), not this.
    Started,
    /// The normal impulse the solver applied to the pair during this tick's step exceeded
    /// [`Collider::impact_threshold`](crate::Collider::impact_threshold) of the target's
    /// collider: an impact, or a sustained crush. Emitted in every such step, whether or
    /// not the pair started or stopped in it. Rapier's `ContactForceEvent`.
    Impact,
    /// The pair stopped touching during this tick's step.
    Stopped,
}

/// Two solid colliders started touching, stopped touching, or pressed harder than the
/// target's impact threshold. Emitted for a pair only if at least one of its colliders
/// has [`Collider::events`](crate::Collider::events) set, twice per pair, once for each
/// body as `target`, so `Events<ContactEvent>::for_target(key)` finds every contact of
/// `key`'s entity. The step emits a tick's events in ascending (`target`, `other`,
/// `phase`) order, whatever order the backend reported them in; at most one event per
/// (`target`, `other`, `phase`) per tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ContactEvent {
    /// The entity the event is for.
    pub target: StableKey,
    /// The entity it touched.
    pub other: StableKey,
    /// Started, impact or stopped.
    pub phase: ContactPhase,
    /// A contact point in world space; zero when stopped.
    pub point: Vec3,
    /// The contact normal, pointing out of `other` towards `target`; zero when stopped.
    pub normal: Vec3,
    /// The total impulse the solver applied along the normal during the step, in mass units
    /// times units per second; zero when stopped, near zero for a speculative start.
    pub impulse: Scalar,
}

impl Event for ContactEvent {
    fn target(&self) -> StableKey {
        self.target
    }
}

/// A sensor collider and another collider started or stopped overlapping. Emitted for a
/// pair only if at least one of its colliders has [`Collider::events`](crate::Collider::events)
/// set, which a sensor has by default; twice per pair, once for each entity as `target`,
/// in ascending (`target`, `other`, `phase`) order after the tick's contact events, at
/// most one per (`target`, `other`, `phase`) per tick. The phase is never
/// [`Impact`](ContactPhase::Impact).
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
