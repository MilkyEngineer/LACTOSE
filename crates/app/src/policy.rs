//! The error policy (architecture sections 3 and 19): what the assembly does with an
//! `Error` from each subsystem. Subsystems return `Result`; this is the one place that
//! decides.

use lactose_core::Error;

/// The `tracing` target of everything this crate logs: `app`, the crate's role name, an
/// exception to the codename rule of architecture section 19 like `io` and `physics`
/// (D119, D130). The assembly has no codename of its own, and the engine's, `lactose`, is
/// a prefix of every `lactose_*` module path: `EnvFilter` matches a directive's target by
/// prefix, so `lactose=debug` would also enable every crate's default-target events. A
/// target no module path starts with can be selected on its own.
pub const LOG_TARGET: &str = "app";

/// Where an error came from, as the policy distinguishes them. A run's `kept` says whether
/// the world came back with the error (`RunError::simulation`, `PredictError::predictor`
/// are `Some`), which is what decides between a resync and a stop.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// Starting the executor or the IO pool, or `Cancelled` from one of the executor's
    /// tasks or sleeps: the executor is shutting down.
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
    /// Encoding a save (`encode_section`, `assemble`) or the slot's `Io::replace`.
    Save,
    /// Decoding a save at start, its sections' validation and migration included.
    Load,
    /// A cooked baseline or asset table a client's fold could not have for a region a
    /// frame made relevant: the client-only asset failure of architecture section 14.
    Cooked,
}

/// What the policy does with an error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Response {
    /// Log it at [`LOG_TARGET`] and carry on: nothing the simulation holds was changed by
    /// the failure.
    Log,
    /// Log it, keep the world, and resync: the prediction world is resynced and a full
    /// frame is requested. A clock resync moves the pacer and the world to the clock target
    /// (`README.md`, "The client's frame"); a content resync keeps the world's own tick,
    /// since the clock was not wrong.
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
    /// save unless `Io::replace` failed after its rename), and a cooked-content failure in
    /// a client's fold resyncs at the world's own tick, so the full frame that follows
    /// retries the region (architecture section 14).
    pub const MILESTONE_1: Self = Self { _private: () };

    /// The response to `error` from `source`. Real, not `todo!()`: the table is the contract.
    #[must_use]
    pub fn respond(&self, source: Source, error: &Error) -> Response {
        match source {
            Source::Executor | Source::Load | Source::Server { .. } => Response::Stop,
            Source::Prediction { kept: true } | Source::Cooked => Response::Resync,
            Source::Prediction { kept: false } => Response::Stop,
            Source::Save => Response::Log,
        }
    }
}
