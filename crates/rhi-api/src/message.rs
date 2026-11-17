//! The message sink: where every validation and driver message a backend receives goes.
//!
//! A message is not an error and changes no call's result. A test counts them and fails
//! on any it did not provoke (milestone 2 plan, decision 5.10); the client logs them.

/// Who said it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MessageSource {
    /// A validator the backend runs under: the Vulkan validation layer's messenger.
    Validation,
    /// The driver, through the backend's own callback or log.
    Driver,
}

/// How bad its source says it is. A test counts both.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// A warning.
    Warning,
    /// An error.
    Error,
}

/// One message.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Message<'a> {
    /// Who said it.
    pub source: MessageSource,
    /// How bad it is.
    pub severity: Severity,
    /// The source's own identifier for the message where it has one (a Vulkan VUID), or
    /// empty.
    pub id: &'a str,
    /// The text.
    pub text: &'a str,
}

/// Where a device's messages go. Called from any thread, a driver's own included, at any
/// time between the constructor's start and the device's drop, and several at once; it
/// must not call the device. A backend passes on every warning and error its sources
/// give it and sends nothing of its own.
///
/// What reaches a process's standard error alone (Metal's API and shader validation) is
/// not delivered here: the command that runs the tests reads it there.
pub trait MessageSink: Send + Sync {
    /// Takes one message.
    fn message(&self, message: &Message<'_>);
}
