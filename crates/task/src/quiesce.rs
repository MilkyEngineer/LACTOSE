//! Quiescence: how a graph run's future is kept from resolving while a job of the run is
//! still executing.
//!
//! Modelled under loom (`models.rs`).
//!
//! One word holds a count of the jobs executing and a closing flag, set when the run has
//! finished its last node or has been cancelled. A job registers and reads the flag in a
//! single atomic step, so there is no window in which a job that saw the run open can start
//! after the run was declared quiet. The run is quiet at the moment the word reads "closing,
//! nobody executing"; whoever brings it there, by leaving or by closing, is told to resolve
//! the future, and every write the jobs made happens before that.

use crate::sync::{AtomicUsize, Ordering};

const CLOSING: usize = 1;
const ONE_JOB: usize = 2;

/// What [`Quiesce::enter`] answers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Entry {
    /// The job is registered and may run. It must call [`Quiesce::exit`] when done.
    Entered,
    /// The run is closing and the job must not run. `quiet` says this refusal found the run
    /// quiet, in which case the caller resolves the future (doing so twice is harmless).
    Refused { quiet: bool },
}

pub(crate) struct Quiesce {
    state: AtomicUsize,
    /// The one change a weakened loom model makes.
    #[cfg(all(test, feature = "loom"))]
    pub(crate) weakened: Option<Weakened>,
}

/// The weakened variants the references' model fails. Each lowers the order of
/// [`Quiesce::exit`], which is what makes a job's writes, and its drop of what it held, happen
/// before the resolution.
#[cfg(all(test, feature = "loom"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Weakened {
    /// [`Quiesce::exit`] releases and does not acquire.
    ExitRelease,
    /// [`Quiesce::exit`] neither releases nor acquires.
    ExitRelaxed,
}

impl Quiesce {
    pub(crate) fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            #[cfg(all(test, feature = "loom"))]
            weakened: None,
        }
    }

    /// Opens the state for a new run. Only called when the previous run is quiet and
    /// resolved, so nothing else is touching the word.
    pub(crate) fn reset(&self) {
        self.state.store(0, Ordering::SeqCst);
    }

    /// Registers a job, unless the run is closing.
    pub(crate) fn enter(&self) -> Entry {
        let previous = self.state.fetch_add(ONE_JOB, Ordering::SeqCst);
        if previous & CLOSING == 0 { Entry::Entered } else { Entry::Refused { quiet: self.exit() } }
    }

    /// Unregisters a job. Returns whether that left the run quiet, in which case the caller
    /// resolves the future.
    #[must_use]
    pub(crate) fn exit(&self) -> bool {
        self.state.fetch_sub(ONE_JOB, self.exit_order()) == ONE_JOB | CLOSING
    }

    /// The order of [`Quiesce::exit`]: each exit releases what its job did, and the one that
    /// finds the run quiet acquires what every earlier exit released.
    fn exit_order(&self) -> Ordering {
        #[cfg(all(test, feature = "loom"))]
        match self.weakened {
            Some(Weakened::ExitRelease) => return Ordering::Release,
            Some(Weakened::ExitRelaxed) => return Ordering::Relaxed,
            None => {}
        }
        Ordering::SeqCst
    }

    /// Unregisters a job as [`Quiesce::exit`] does, once `held`, the job's reference to what
    /// the run works on, has been dropped. Returns whether that left the run quiet, in which
    /// case the caller resolves the future: no job holds such a reference by then.
    #[must_use]
    pub(crate) fn leave<T>(&self, held: T) -> bool {
        // First: a job that has left may be passed by the last to leave, which resolves the
        // future at once, and the caller then drops what it was handed back.
        drop(held);
        self.exit()
    }

    /// Declares that no further job may start. Returns whether the run is quiet already, in
    /// which case the caller resolves the future; otherwise the last job to leave does.
    #[must_use]
    pub(crate) fn close(&self) -> bool {
        self.state.fetch_or(CLOSING, Ordering::SeqCst) == 0
    }

    /// Whether the run is closing. A registered job reads this between units to stop early;
    /// it is only a hint there, since the job's registration is what holds the future back.
    pub(crate) fn closing(&self) -> bool {
        self.state.load(Ordering::Relaxed) & CLOSING != 0
    }
}
