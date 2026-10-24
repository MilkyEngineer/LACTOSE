//! The plain data a request carries and returns: how a stored range is decoded, what a
//! directory listing holds, and the one size cap.

use std::ffi::OsString;

/// The most bytes one read may ask for, stored or decoded: 1 GiB.
///
/// A range or a `decoded_len` above it fails with `ErrorKind::ResourceExhausted` before
/// anything is allocated, so a corrupt pack index or save index asking for an absurd
/// length is an error value and never an allocation failure (architecture section 19).
/// Larger content is read in ranges.
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
