//! Thread creation and core topology behind a trait, so consoles with fixed affinity can
//! replace both without touching the executor.

use lactose_core::{Result, ThreadClass};

/// How many cores of each kind the executor may use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoreTopology {
    /// Performance cores. On hardware without core-type information this is the physical
    /// core count minus one.
    pub performance: u32,
    /// Efficiency cores. Zero on homogeneous CPUs, where the executor runs two low-priority
    /// background threads instead.
    pub efficiency: u32,
}

/// A request for one thread.
#[derive(Clone, Debug)]
pub struct ThreadSpec {
    /// The thread name shown in debuggers and profilers.
    pub name: String,
    /// Scheduling class and floating-point state; the platform applies it with
    /// [`ThreadClass::enter`] or its own equivalent before running the body.
    pub class: ThreadClass,
    /// The position of this thread among those of its class, for platforms that pin.
    pub index_in_class: u32,
}

/// A thread created by a [`Platform`].
pub struct ThreadHandle {
    join: Box<dyn FnOnce() + Send>,
}

impl ThreadHandle {
    /// Wraps the function that waits for the thread to finish.
    #[must_use]
    pub fn new(join: impl FnOnce() + Send + 'static) -> Self {
        Self { join: Box::new(join) }
    }

    /// Waits for the thread to finish.
    pub fn join(self) {
        (self.join)();
    }
}

/// The operating-system services the executor needs.
pub trait Platform: Send + Sync + 'static {
    /// The cores available to the engine.
    fn topology(&self) -> CoreTopology;

    /// Starts a thread that runs `body` after applying `spec.class`.
    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle>;
}

/// The default platform: `std::thread`, with topology from `hw.perflevel0.physicalcpu` on
/// macOS, the hybrid core-type query on Windows and CPU capacity from sysfs on Linux.
#[derive(Clone, Copy, Default, Debug)]
pub struct StdPlatform;

impl Platform for StdPlatform {
    fn topology(&self) -> CoreTopology {
        todo!()
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        todo!()
    }
}
