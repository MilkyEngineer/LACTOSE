//! Timelines and the submit.
//!
//! A timeline is a counter that only grows. Submits wait for its values and signal them,
//! the host signals, waits and reads it, and work crosses queues by it on every backend.
//! A fence is a timeline point: the facade has no other fence (`README.md`, "Timelines").

use lactose_core::Handle;

use crate::encoder::Encoder;
use crate::resource::kind;

/// A timeline of a device.
pub type TimelineHandle = Handle<kind::Timeline>;

/// A value of a timeline: reached once the timeline's value is at least `value`. A wait
/// for a point orders the signal that reaches it and what is ordered before that signal,
/// among it the earlier submits of the signal's queue, and not the other signals of its
/// submit (`README.md`, "Timelines").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimelinePoint {
    /// The timeline.
    pub timeline: TimelineHandle,
    /// The value.
    pub value: u64,
}

/// One submit: encoders recorded on any threads, run in slice order, after every point of
/// `waits` is reached; when all of them have run, every point of `signals` is reached and
/// the submit has completed. Until then it is pending.
///
/// A submit takes all of its encoders or none. Refused, it leaves them recorded as they
/// were, changes no state and signals nothing.
pub struct Submit<'a> {
    /// The encoders, each recording, each of the submit's role and of this device. A
    /// submit that takes them makes them pending; an encoder that recorded nothing adds
    /// nothing, and a submit with none waits and signals alone.
    pub encoders: &'a mut [Box<dyn Encoder>],
    /// The points the device waits for before it runs anything of the submit.
    pub waits: &'a [TimelinePoint],
    /// The points reached when the submit completes: each value greater than any its
    /// timeline has reached or been promised by another submit or signal, and each
    /// ordered after every other queue's pending signal of its timeline by a wait for that
    /// signal or for a signal of a later submit of its queue
    /// ([`Rule::Timeline`](crate::Rule::Timeline)). The points are unordered with one
    /// another: a wait for one of them orders none of the others.
    pub signals: &'a [TimelinePoint],
}
