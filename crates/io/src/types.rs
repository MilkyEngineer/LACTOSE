//! The plain data a request carries and returns: how a stored range is decoded, what a
//! directory listing holds, what a failed replace hands back, and the one size cap.

use std::ffi::OsString;
use std::fmt;

use lactose_core::Error;

/// The most bytes one request may ask for, stored or decoded: 1 GiB.
///
/// A range, a `decoded_len` or a whole file above it fails with
/// `ErrorKind::ResourceExhausted` before anything is allocated, so a corrupt pack index or
/// save index asking for an absurd length is an error value (architecture section 19). The
/// cap is per request: requests in flight at once may each hold up to this much. Larger
/// content is read in ranges.
pub const MAX_READ_BYTES: u64 = 1 << 30;

/// How the bytes of a stored range are decoded before they are handed back.
///
/// A read that decodes is one request, not a read followed by a decode, so a platform with
/// hardware decompression can serve it without the caller changing
/// (`docs/verification.md`, section 3). The caller knows the decoded length from the index
/// that named the range; a frame that decodes to any other length is `InvalidData`.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Encoding {
    /// One zstd frame whose decoded length is `decoded_len`.
    Zstd {
        /// The exact number of bytes the frame decodes to, from the index that named the
        /// range. At most [`MAX_READ_BYTES`].
        decoded_len: u64,
    },
}

/// One entry of a directory listing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DirEntry {
    /// The entry's own name, without the directory. Joined onto the listed path to open it.
    pub name: OsString,
    /// What the entry is, by its own type: a symbolic link is `Other`, whatever it points to.
    pub kind: EntryKind,
}

/// What a directory entry is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// Anything else: a symbolic link, a device, a socket, a pipe.
    Other,
}

/// A failed [`Io::replace`](crate::Io::replace): the error, and the bytes it was given,
/// handed back unchanged and never copied so a retry needs no copy of its own.
///
/// Converts into its `error` with `?` where only the error matters.
#[non_exhaustive]
pub struct ReplaceFailed {
    /// What went wrong; its kind says whether `path` may hold the new content
    /// (`README.md`, "The atomic replace").
    pub error: Error,
    /// The bytes the replace was given, as they were.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for ReplaceFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplaceFailed")
            .field("error", &self.error)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

impl fmt::Display for ReplaceFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

/// Transparent: the display and the source are the error's own.
impl std::error::Error for ReplaceFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.error)
    }
}

impl From<ReplaceFailed> for Error {
    fn from(failed: ReplaceFailed) -> Self {
        failed.error
    }
}
