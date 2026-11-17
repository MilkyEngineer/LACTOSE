//! The thread and topology provider of a client binary.

use lactose_core::Result;
use lactose_task::{CoreTopology, Platform, ThreadHandle, ThreadSpec};

/// The `lactose_task::Platform` of a client binary: what it passes as `Workers::platform`,
/// and so what makes the executor's workers, the IO pool's threads and the present thread.
///
/// It reports the cores `StdPlatform` reports and makes a thread as `StdPlatform` makes
/// one. What it adds is the operating system's own treatment of a thread that presents,
/// for `ThreadClass::Present` alone (`README.md`, "`ClientPlatform`").
#[derive(Clone, Debug, Default)]
pub struct ClientPlatform {
    /// Nothing yet; a value is made through `new` or `default` alone.
    private: (),
}

impl ClientPlatform {
    /// The client's platform. It may be made on any thread, before or after the event
    /// pump, and more than once: every value behaves alike.
    #[must_use]
    pub const fn new() -> Self {
        Self { private: () }
    }
}

impl Platform for ClientPlatform {
    /// What `StdPlatform::topology` returns in the same process.
    fn topology(&self) -> CoreTopology {
        todo!()
    }

    /// What `StdPlatform::hardware_threads` returns in the same process.
    fn hardware_threads(&self) -> Option<u32> {
        todo!()
    }

    /// Starts a thread named `spec.name` that enters `spec.class` and then runs `body`.
    /// A class the operating system refuses is a warning on [`LOG_TARGET`](crate::LOG_TARGET),
    /// and the thread runs at its default priority.
    ///
    /// # Errors
    ///
    /// The operating system's refusal to make a thread, as `StdPlatform::spawn_thread`
    /// reports it; `body` is then dropped on the calling thread.
    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        todo!()
    }
}
