//! Chaos scheduling for the determinism harness (architecture section 8.3).

/// Perturbs scheduling to expose ordering bugs that a four-core CI runner never hits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ChaosConfig {
    /// Seeds every perturbation, so a run repeats given the same seed and worker count.
    pub seed: u64,
    /// Choose steal victims and steal timing from the seed.
    pub shuffle_steals: bool,
    /// Yield the worker's time slice at seeded points between jobs.
    pub inject_yields: bool,
    /// Split every range job down to single units, as if every half had been stolen.
    pub force_split: bool,
    /// Record which worker ran which job in which order, for
    /// [`crate::ExecutorBuilder::replay`].
    pub record: bool,
}

/// A recorded job execution order. Opaque; it round-trips through bytes so a failing CI run
/// can attach it as an artifact.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExecutionLog {
    bytes: Vec<u8>,
}

impl ExecutionLog {
    /// Rebuilds a log from [`ExecutionLog::as_bytes`] output.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The serialised log.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
