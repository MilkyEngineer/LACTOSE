//! A lane whose thread parks while nothing is queued: a thread that runs nothing but its
//! lane's calls, as the present thread does.
//!
//! The wake is a flag beside the unpark. A call that parks inside its own body, on a channel
//! or a fence's wait, can take the unpark of a call queued while it runs, and a thread that
//! then parked on the unpark alone would wait for the next call's. The flag is written
//! against `sync`, so the model `a_parked_lane_loses_no_wake_to_a_call_that_parks` checks
//! the shipped protocol.

use std::sync::Arc;

use crate::lane::{Lane, LanePump, lane};
use crate::sync::{AtomicBool, Ordering, Thread, current, park};

/// Creates a lane drained by the calling thread, which parks in [`ParkingPump::wait`] until
/// a call is queued. The pump is `!Send`, as every [`LanePump`] is, so the thread that
/// creates the lane is the one that drains it and the one a queued call unparks.
pub fn parking_lane<S: 'static>(name: &'static str) -> (Lane<S>, ParkingPump<S>) {
    let signal = Arc::new(Signal { queued: AtomicBool::new(false), thread: current() });
    let (lane, pump) = {
        let signal = signal.clone();
        lane(name, move || signal.wake())
    };
    (lane, ParkingPump { pump, signal })
}

/// The receiving side of a [`parking_lane`]. Dropping it closes the lane, as dropping a
/// [`LanePump`] does.
pub struct ParkingPump<S> {
    pump: LanePump<S>,
    signal: Arc<Signal>,
}

impl<S: 'static> ParkingPump<S> {
    /// [`LanePump::drain`]: runs every call that was queued when it was called, and returns
    /// how many ran.
    pub fn drain(&mut self, state: &mut S) -> usize {
        self.pump.drain(state)
    }

    /// Parks the calling thread until a call has been queued since the last `wait` returned,
    /// or since the lane was made; returns at once if one has. An unpark from anywhere else
    /// does not end it, and a call that took the thread's unpark does not lose its wake.
    /// The call is not run: the caller drains after it.
    pub fn wait(&self) {
        // Both sides swap: loom 0.7.2 lets a read-modify-write read past a plain store that
        // nothing orders after it, which no read-modify-write may do at any ordering, and
        // against a swap it cannot (`sync`, `TimedPark`). The queue's mutex orders the call
        // itself, so the flag publishes nothing.
        while !self.signal.queued.swap(false, Ordering::Acquire) {
            park();
        }
    }
}

/// What a queued call sets, and the thread it unparks.
struct Signal {
    queued: AtomicBool,
    thread: Thread,
}

impl Signal {
    /// The lane's `wake`: called by `Lane::run` after the call is queued and the queue's
    /// lock released.
    fn wake(&self) {
        self.queued.swap(true, Ordering::Release);
        self.thread.unpark();
    }
}
