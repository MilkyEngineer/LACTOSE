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
        // against a swap it cannot (`sync`, `TimedPark`). The Release and Acquire here and in
        // `wake` are deliberate, and nothing relies on them: the queue's lock orders the
        // calls, so the flag publishes nothing.
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

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    /// Long enough for a loaded machine, and for Miri; a lost wake fails with a message.
    const BOUND: Duration = Duration::from_secs(if cfg!(miri) { 3600 } else { 30 });

    /// The loom model's case on real threads: a call parks its thread until the next call
    /// is queued, takes that call's unpark, and the thread then waits for the next call
    /// without losing it.
    #[test]
    fn a_call_queued_while_a_call_parks_runs_after_it() {
        let (lanes, lent) = mpsc::channel();
        let (ran, done) = mpsc::channel();
        let pumping = std::thread::spawn(move || {
            let (lane, mut pump) = super::parking_lane::<usize>("parking::tests");
            lanes.send(lane).expect("the test waits for the lane");
            let mut count = 0;
            loop {
                pump.drain(&mut count);
                if count == 2 {
                    break;
                }
                pump.wait();
            }
            ran.send(count).expect("the test waits for the count");
        });
        let lane = lent.recv_timeout(BOUND).expect("the lane is made");
        let second = Arc::new(AtomicBool::new(false));
        let (entered, inside) = mpsc::channel();
        let parking = {
            let second = second.clone();
            lane.run(move |count: &mut usize| {
                entered.send(()).expect("the test waits for the call");
                while !second.load(Ordering::SeqCst) {
                    std::thread::park();
                }
                *count += 1;
            })
        };
        inside.recv_timeout(BOUND).expect("the parking call started");
        second.store(true, Ordering::SeqCst);
        drop(lane.run(|count: &mut usize| *count += 1));
        assert_eq!(done.recv_timeout(BOUND).expect("the call behind the parking one ran"), 2);
        pumping.join().expect("the pump ends");
        drop(parking);
    }
}
