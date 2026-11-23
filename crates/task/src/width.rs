//! The width: how many frame-critical workers the jobs of [`crate::Executor::run_job`] and the
//! children of their scopes occupy at most (README, "The width").
//!
//! Modelled under loom (`models.rs`), so everything here goes through `sync`.
//!
//! Such work waits on two queues of the pool's that hold nothing else, one for the jobs and
//! one for their scopes' children, and never on a worker's own deque. One word holds the
//! number of places taken and a flag that says a job holds one of them. A worker that holds
//! no place takes from a queue only after raising the count while it is below the width, and
//! a job only while no other job holds a place, so the jobs run one after the other and every
//! child queued belongs to the one scope that is open. The opener of that scope takes its
//! children under the place its job holds, without raising the count.
//!
//! Two pairs keep a queued entry from waiting while a place is free. A worker that queues an
//! entry ([`Width::queued`]) and a worker that gives a place up ([`Width::leave`]) each write
//! one word, then read the other's, and wake a sleeper if what they read leaves an entry
//! takeable: the queue and then the count, the count and then the queue. That is a
//! store-buffering pattern, so each side has a `SeqCst` fence between its write and its read,
//! and one of the two always sees the other's write. Release and acquire would let both read
//! the old values, and the entry would wait beside a sleeping worker until something else
//! woke one. The queues are `crossbeam-deque` injectors, which publish an entry with orderings
//! of their own, which is why these fences do not lean on them; the sleeper's side is the
//! idle protocol's (`sleep.rs`), whose look counts these queues only while [`Width::admits`]
//! says a worker without a place could take from them.

use crate::gate::Wake;
use crate::sync::{AtomicUsize, Ordering, fence};

/// Set in the word while a job holds a place.
const JOB: usize = 1 << (usize::BITS - 1);

/// What a place is taken for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Place {
    /// A job of `run_job`, from its start until its closure has returned.
    Job,
    /// One entry of a child of such a job's scope, taken by a worker that holds no place.
    Child,
}

/// Whether the two queues hold anything. The pool reads its injectors; a model its counters.
pub(crate) trait Queued {
    fn jobs(&self) -> bool;
    fn children(&self) -> bool;
}

/// The places of one pool.
pub(crate) struct Width {
    limit: usize,
    /// The places taken, and [`JOB`].
    state: AtomicUsize,
    /// The one change a weakened loom model makes.
    #[cfg(all(test, feature = "loom"))]
    pub(crate) weakened: Option<Weakened>,
}

/// The weakened variants the models fail. Each changes one thing in this file.
#[cfg(all(test, feature = "loom"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Weakened {
    /// [`Width::queued`] reads the count without its fence.
    PushFence,
    /// [`Width::leave`] reads the queues without its fence.
    LeaveFence,
    /// Both fences are `AcqRel`.
    AcqRelFences,
    /// [`Width::try_enter`] reads the count, then raises it whatever it has become.
    SplitRaise,
    /// A job raises the count as a child does and ignores [`JOB`].
    NoJobFlag,
}

/// Which of the two fences [`Width::fence`] issues.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Push,
    Leave,
}

impl Width {
    /// The width of a pool of `workers` frame-critical workers: all but one, and one where
    /// that is one.
    pub(crate) fn new(workers: usize) -> Self {
        Self {
            limit: workers.saturating_sub(1).max(1),
            state: AtomicUsize::new(0),
            #[cfg(all(test, feature = "loom"))]
            weakened: None,
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    fn job_flag(&self, place: Place) -> usize {
        #[cfg(all(test, feature = "loom"))]
        if self.weakened == Some(Weakened::NoJobFlag) {
            return 0;
        }
        if place == Place::Job { JOB } else { 0 }
    }

    /// Whether a worker that holds no place may take one for `place` in `state`.
    fn free(&self, state: usize, place: Place) -> bool {
        state & !JOB < self.limit && state & self.job_flag(place) == 0
    }

    /// Takes a place for `place` if one is free. The caller then takes from the queue and
    /// gives the place up with [`Width::leave`] when the entry is done, or at once if the
    /// queue turned out to be empty.
    pub(crate) fn try_enter(&self, place: Place) -> bool {
        let mut state = self.state.load(Ordering::SeqCst);
        loop {
            if !self.free(state, place) {
                return false;
            }
            let raised = (state + 1) | self.job_flag(place);
            #[cfg(all(test, feature = "loom"))]
            if self.weakened == Some(Weakened::SplitRaise) {
                self.state.fetch_add(raised - state, Ordering::SeqCst);
                return true;
            }
            match self.state.compare_exchange_weak(
                state,
                raised,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => state = actual,
            }
        }
    }

    /// Gives up a place taken for `place`, then wakes a sleeper if an entry is queued that a
    /// worker without a place may now take. The lowering comes before the read of the queues,
    /// with a fence between them: the other half of [`Width::queued`]'s pair.
    pub(crate) fn leave(&self, place: Place, queued: &impl Queued, wake: &impl Wake) {
        self.state.fetch_sub(1 | self.job_flag(place), Ordering::SeqCst);
        self.fence(Side::Leave);
        if self.admits(queued) {
            wake.idle_one();
        }
    }

    /// What a worker does after it queues an entry for `place`: tells the scope's `opener`,
    /// when the entry is a child and the caller is another worker, then wakes a sleeper if a
    /// worker without a place may take the entry now. The queueing comes before the read of
    /// the count, with a fence between them: the other half of [`Width::leave`]'s pair. A
    /// sleeper that is woken while the count is full would find nothing it may take.
    pub(crate) fn queued(&self, place: Place, opener: Option<usize>, wake: &impl Wake) {
        // The opener takes its scope's children under its job's place, whatever the count,
        // and reads the queue after every wake (`gate::wait_for_children`).
        if let Some(opener) = opener {
            wake.unpark(opener);
        }
        self.fence(Side::Push);
        if self.free(self.state.load(Ordering::SeqCst), place) {
            wake.idle_one();
        }
    }

    /// Whether a worker that holds no place could take something queued now. An idle
    /// worker's last look counts the queues only when this says so.
    pub(crate) fn admits(&self, queued: &impl Queued) -> bool {
        let state = self.state.load(Ordering::SeqCst);
        (self.free(state, Place::Child) && queued.children())
            || (self.free(state, Place::Job) && queued.jobs())
    }

    /// How many places are taken.
    #[cfg(test)]
    pub(crate) fn occupied(&self) -> usize {
        self.state.load(Ordering::SeqCst) & !JOB
    }

    fn fence(&self, side: Side) {
        #[cfg(all(test, feature = "loom"))]
        match (self.weakened, side) {
            (Some(Weakened::PushFence), Side::Push) | (Some(Weakened::LeaveFence), Side::Leave) => {
                return;
            }
            (Some(Weakened::AcqRelFences), _) => return fence(Ordering::AcqRel),
            _ => {}
        }
        let _ = side;
        fence(Ordering::SeqCst);
    }
}
