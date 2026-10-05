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
}

impl Quiesce {
    pub(crate) fn new() -> Self {
        Self { state: AtomicUsize::new(0) }
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
        self.state.fetch_sub(ONE_JOB, Ordering::SeqCst) == ONE_JOB | CLOSING
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
