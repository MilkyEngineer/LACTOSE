//! The thread and topology provider of a client binary.

use lactose_core::Result;
use lactose_task::{CoreTopology, Platform, StdPlatform, ThreadHandle, ThreadSpec};

use crate::LOG_TARGET;

/// The `lactose_task::Platform` of a client binary: what it passes as `Workers::platform`,
/// and so what makes the executor's workers, the IO pool's threads and the present thread.
///
/// It reports the cores `StdPlatform` reports and makes a thread as `StdPlatform` makes
/// one. What it may add is the operating system's own treatment of a thread that presents,
/// for `ThreadClass::Present` alone (`README.md`, "`ClientPlatform`"), and it adds none:
/// `ThreadClass::enter` gives that class macOS's highest quality of service and Windows's
/// above-normal priority, and no milestone 2 measurement asks for more.
#[derive(Clone, Debug, Default)]
pub struct ClientPlatform {
    /// Nothing: a field no other crate can name, so that a value is made through `new` or
    /// `default` alone.
    #[expect(dead_code, reason = "it holds nothing, and is never read")]
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
        StdPlatform.topology()
    }

    /// What `StdPlatform::hardware_threads` returns in the same process.
    fn hardware_threads(&self) -> Option<u32> {
        StdPlatform.hardware_threads()
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
        let class = spec.class;
        let thread = std::thread::Builder::new().name(spec.name).spawn(move || {
            // Miri cannot call the scheduling interfaces, and has nothing to schedule.
            if !cfg!(miri)
                && let Err(error) = class.enter()
            {
                tracing::warn!(target: LOG_TARGET, %error, ?class, "thread class not applied");
            }
            body();
        })?;
        Ok(ThreadHandle::new(move || {
            // A panic on the thread was reported by the panic hook, and a binary aborts on
            // one: there is nothing to add here.
            let _ = thread.join();
        }))
    }
}
