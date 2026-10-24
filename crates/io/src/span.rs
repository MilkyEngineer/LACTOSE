//! The one span every request emits (architecture section 3, "Traced from day one").
//!
//! The span is created with the request's future, entered on the pool thread for the
//! blocking work, and closed when the request's state is released, so a profiler sees the
//! time waiting and the time working apart. The names and fields below are the contract
//! `README.md` states; a profiler configuration may rely on them.

use std::path::Path;

use tracing::field::Empty;
use tracing::{Level, Span};

use crate::Encoding;

/// The log target of everything this crate emits.
pub(crate) const TARGET: &str = "io";

/// The level of a request span. Finer levels compile out, as for the executor's spans.
pub(crate) const LEVEL: Level = Level::INFO;

/// `io.open`: `path`, and on completion `error`.
pub(crate) fn open(path: &Path) -> Span {
    tracing::span!(target: TARGET, LEVEL, "io.open", path = %path.display(), error = Empty)
}

/// `io.read`: `file` (the handle's id), `offset`, `len`, `encoding` (`"raw"` or `"zstd"`)
/// and `decoded_len` (equal to `len` for a raw read), and on completion `error`.
pub(crate) fn read(file: u64, offset: u64, len: u64, encoding: Option<Encoding>) -> Span {
    let (encoding, decoded_len) = match encoding {
        None => ("raw", len),
        Some(Encoding::Zstd { decoded_len }) => ("zstd", decoded_len),
    };
    tracing::span!(
        target: TARGET,
        LEVEL,
        "io.read",
        file,
        offset,
        len,
        encoding,
        decoded_len,
        error = Empty
    )
}

/// `io.read_file`: `path`, and on completion `len` (the bytes read) and `error`.
pub(crate) fn read_file(path: &Path) -> Span {
    tracing::span!(
        target: TARGET,
        LEVEL,
        "io.read_file",
        path = %path.display(),
        len = Empty,
        error = Empty
    )
}

/// `io.replace`: `path` and `len` (the bytes written), and on completion `error`.
pub(crate) fn replace(path: &Path, len: u64) -> Span {
    tracing::span!(target: TARGET, LEVEL, "io.replace", path = %path.display(), len, error = Empty)
}

/// `io.read_dir`: `path`, and on completion `entries` and `error`.
pub(crate) fn read_dir(path: &Path) -> Span {
    tracing::span!(
        target: TARGET,
        LEVEL,
        "io.read_dir",
        path = %path.display(),
        entries = Empty,
        error = Empty
    )
}

/// `io.create_dir_all`: `path`, and on completion `error`.
pub(crate) fn create_dir_all(path: &Path) -> Span {
    tracing::span!(
        target: TARGET,
        LEVEL,
        "io.create_dir_all",
        path = %path.display(),
        error = Empty
    )
}
