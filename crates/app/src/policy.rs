//! The error policy (architecture sections 3 and 19): what the assembly does with an
//! `Error` from each subsystem. Subsystems return `Result`; this is the one place that
//! decides.

use lactose_core::Error;

/// The `tracing` target of everything this crate logs: the engine's own codename, since the
/// assembly is the engine put together and has no codename of its own (architecture section
/// 23).
pub const LOG_TARGET: &str = "lactose";

/// Where an error came from, as the policy distinguishes them. A run's `kept` says whether
/// the world came back with the error (`RunError::simulation`, `PredictError::predictor`
/// are `Some`), which is what decides between a resync and a stop.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// Starting the executor, or `Cancelled` from one of its tasks or sleeps: the executor
    /// is shutting down.
    Executor,
    /// A server world's run: `step`, `run_passes`, `save_point` or the hash.
    Server {
        /// Whether the simulation came back with the error.
        kept: bool,
    },
    /// The prediction world's run: `step`, `reconcile`, `passes` or the hash.
    Prediction {
        /// Whether the predictor came back with the error.
        kept: bool,
    },
    /// Encoding a save (`from_export`, `encode_section`, `assemble`) or writing the slot.
    Save,
    /// Decoding a save at start, its sections' bundles included.
    Load,
    /// A cooked baseline or asset table a client's fold could not have for a region a
    /// frame made relevant.
    Cooked,
}

/// What the policy does with an error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Response {
    /// Log it at [`LOG_TARGET`] and carry on: nothing the simulation holds was changed by
    /// the failure.
    Log,
    /// Log it, keep the world, and resync: the client's pacer and predictor move to the
    /// server's tick plus the lead, and a full frame is requested.
    Resync,
    /// Log it and stop: the run that failed ends with the error, which reaches the binary's
    /// `main`. Startup failures are these, and so is every failure of a server world, since
    /// a server never rolls back and a prepare failure means an unjournaled edit removed
    /// what a system names.
    Stop,
}

/// The policy: a table from [`Source`] to [`Response`], one per milestone. The milestone 1
/// table reads no error kind; milestone 4's reads `StorageFull` on a save (the disk-full
/// path), `InvalidData` and `VersionMismatch` on a load (the corrupt-save path) and
/// `Network` from the transport (the drop-and-reconnect path), each a defined path by then
/// (architecture section 19).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ErrorPolicy {
    _private: (),
}

impl ErrorPolicy {
    /// Milestone 1: every startup and server failure stops, a kept prediction failure
    /// resyncs, a torn one stops, a save failure is logged (the slot keeps its previous
    /// save, since nothing was written) and a cooked-content failure in a client's fold is
    /// logged (the region is left out of that frame; the next full frame retries it).
    pub const MILESTONE_1: Self = Self { _private: () };

    /// The response to `error` from `source`. Real, not `todo!()`: the table is the contract.
    #[must_use]
    pub fn respond(&self, source: Source, error: &Error) -> Response {
        match source {
            Source::Executor | Source::Load | Source::Server { .. } => Response::Stop,
            Source::Prediction { kept: true } => Response::Resync,
            Source::Prediction { kept: false } => Response::Stop,
            Source::Save | Source::Cooked => Response::Log,
        }
    }
}
