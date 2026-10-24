//! The one span every request emits (architecture section 3, "Traced from day one").
//!
//! The span is created at the future's first poll, when the request is submitted, entered
//! on the pool thread for the blocking work and by the future's poll for each decode slice,
//! and closed when the request's state is released. A future dropped unpolled creates none.
//! The names and fields below are the contract `README.md` states; a profiler configuration
//! may rely on them.

use std::path::Path;

use tracing::field::Empty;
use tracing::{Level, Span};

use crate::Encoding;
use crate::file::FileId;

/// The log target of everything this crate emits: the role name, since `io` has no codename
/// (D130, the exception to architecture section 19 that `physics` set).
pub(crate) const TARGET: &str = "io";

/// The level of a request span. Finer levels compile out, as for the executor's spans.
pub(crate) const LEVEL: Level = Level::INFO;

/// `io.open`: `path` and `file` (the id the handle will carry, allocated by `Io::open`),
/// and on completion `size` and `error`.
pub(crate) fn open(path: &Path, file: FileId) -> Span {
    tracing::span!(
        target: TARGET,
        LEVEL,
        "io.open",
        path = %path.display(),
        file = file.0,
        size = Empty,
        error = Empty
    )
}

/// `io.read`: `file` (the handle's id), `offset`, `len`, `encoding` (`"raw"` or `"zstd"`)
/// and `decoded_len` (equal to `len` for a raw read), and on completion `error`.
pub(crate) fn read(file: FileId, offset: u64, len: u64, encoding: Option<Encoding>) -> Span {
    let (encoding, decoded_len) = match encoding {
        None => ("raw", len),
        Some(Encoding::Zstd { decoded_len }) => ("zstd", decoded_len),
    };
    tracing::span!(
        target: TARGET,
        LEVEL,
        "io.read",
        file = file.0,
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
