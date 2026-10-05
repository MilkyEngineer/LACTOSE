//! The error taxonomy, architecture section 19.
//!
//! Subsystems return [`Result`]. Policy (retry, resync, surface to the player, shut down)
//! belongs to `app` and is chosen from the [`ErrorKind`], never from the message.

use std::borrow::Cow;

/// What went wrong, at the granularity policy needs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A named thing (asset, file, save slot, entity) does not exist.
    NotFound,
    /// Bytes from disk or the network failed validation.
    InvalidData,
    /// A schema, protocol, build or content hash did not match.
    VersionMismatch,
    /// A file operation failed for a reason other than the ones listed separately.
    Io,
    /// The disk or save quota is full.
    StorageFull,
    /// The connection dropped, timed out or was refused.
    Network,
    /// The graphics device was lost and its resources must be rebuilt.
    DeviceLost,
    /// A budget or capacity (memory, handles, slots) is exhausted.
    ResourceExhausted,
    /// The platform or backend lacks a capability.
    Unsupported,
    /// The operation was cancelled before it completed.
    Cancelled,
    /// An operating-system call failed.
    Platform,
}

impl ErrorKind {
    /// Every kind with its code, in code order. Codes are never reused or renumbered.
    const CODES: [(u8, Self); 11] = [
        (1, Self::NotFound),
        (2, Self::InvalidData),
        (3, Self::VersionMismatch),
        (4, Self::Io),
        (5, Self::StorageFull),
        (6, Self::Network),
        (7, Self::DeviceLost),
        (8, Self::ResourceExhausted),
        (9, Self::Unsupported),
        (10, Self::Cancelled),
        (11, Self::Platform),
    ];

    /// The stable code of this kind, for journals, saves and the wire. Zero is never a
    /// code, so it is free to mean "no error" in plain data.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        let mut i = 0;
        while i < Self::CODES.len() {
            if Self::CODES[i].1 as u8 == self as u8 {
                return Self::CODES[i].0;
            }
            i += 1;
        }
        panic!("every ErrorKind has a code")
    }

    /// The kind with the code `code`, or `None` for a code this build does not know.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        let mut i = 0;
        while i < Self::CODES.len() {
            if Self::CODES[i].0 == code {
                return Some(Self::CODES[i].1);
            }
            i += 1;
        }
        None
    }
}

/// A recoverable failure. Broken invariants panic instead and never become an `Error`.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: Cow<'static, str>,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

/// The result type every subsystem returns.
pub type Result<T, E = Error> = core::result::Result<T, E>;

impl Error {
    /// An error of `kind` described by `message`.
    pub fn new(kind: ErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self { kind, message: message.into(), source: None }
    }

    /// Attaches the lower-level error that caused this one.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// The kind policy dispatches on.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The human-readable description.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        use std::io::ErrorKind as Io;
        let kind = match error.kind() {
            Io::NotFound => ErrorKind::NotFound,
            Io::InvalidData | Io::UnexpectedEof => ErrorKind::InvalidData,
            Io::StorageFull | Io::QuotaExceeded => ErrorKind::StorageFull,
            Io::OutOfMemory => ErrorKind::ResourceExhausted,
            Io::Unsupported => ErrorKind::Unsupported,
            Io::ConnectionRefused
            | Io::ConnectionReset
            | Io::ConnectionAborted
            | Io::NotConnected
            | Io::TimedOut => ErrorKind::Network,
            _ => ErrorKind::Io,
        };
        Self::new(kind, error.to_string()).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_policy_kinds() {
        let missing: Error = std::io::Error::from(std::io::ErrorKind::NotFound).into();
        assert_eq!(missing.kind(), ErrorKind::NotFound);
        let full: Error = std::io::Error::from(std::io::ErrorKind::StorageFull).into();
        assert_eq!(full.kind(), ErrorKind::StorageFull);
        let other: Error = std::io::Error::from(std::io::ErrorKind::PermissionDenied).into();
        assert_eq!(other.kind(), ErrorKind::Io);
        assert!(std::error::Error::source(&other).is_some());
    }

    /// Golden codes. A journal or save written by one build is read by another, so a code
    /// never changes; a new kind takes the next free code.
    #[test]
    fn kind_codes_are_pinned() {
        use ErrorKind::*;
        let pinned = [
            (NotFound, 1),
            (InvalidData, 2),
            (VersionMismatch, 3),
            (Io, 4),
            (StorageFull, 5),
            (Network, 6),
            (DeviceLost, 7),
            (ResourceExhausted, 8),
            (Unsupported, 9),
            (Cancelled, 10),
            (Platform, 11),
        ];
        for (kind, code) in pinned {
            // Fails to compile when a kind is added without a code and a line here.
            match kind {
                NotFound | InvalidData | VersionMismatch | Io | StorageFull | Network
                | DeviceLost | ResourceExhausted | Unsupported | Cancelled | Platform => {}
            }
            assert_eq!(kind.to_u8(), code);
            assert_eq!(ErrorKind::from_u8(code), Some(kind));
        }
        for unknown in [0, 12, 255] {
            assert_eq!(ErrorKind::from_u8(unknown), None);
        }
    }

    #[test]
    fn display_names_kind_and_message() {
        let error = Error::new(ErrorKind::VersionMismatch, "protocol hash differs");
        assert_eq!(error.to_string(), "VersionMismatch: protocol hash differs");
        assert_eq!(error.message(), "protocol hash differs");
    }

    #[test]
    fn error_crosses_threads() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
    }
}
