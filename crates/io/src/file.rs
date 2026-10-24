//! An open file and the positional reads it serves.

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use lactose_core::Result;

use crate::backend::Backend;
use crate::decode::Decode;
use crate::request::{Encoded, Pending};
use crate::types::Encoding;

/// The number a pool gives each file it opens, for the `file` field of its read spans.
///
/// Unique within one [`Io`](crate::Io) for its lifetime, never reused, and not dense: it is
/// taken when [`Io::open`](crate::Io::open) is called, so an open dropped unpolled or failed
/// leaves a gap. The `io.open` span carries it from its creation, so a profiler joins every
/// `io.read` to the open that made the handle. Nothing else is derived from it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FileId(pub(crate) u64);

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A file open for reading, shared by every clone and every read in flight on it.
///
/// Opened by [`Io::open`](crate::Io::open). Cloning shares the one descriptor; the
/// descriptor closes when the last clone and the last read in flight on it are gone,
/// on whichever thread drops it last, without a request. Reads are positional and
/// independent, so any number may be in flight at once on the same handle. On Windows the
/// blocking backend's handle is synchronous, and the system runs the reads on one handle one
/// at a time; reads on two opens of one path do not wait for each other (`README.md`).
#[derive(Clone)]
pub struct File {
    inner: Arc<OpenFile>,
}

/// The descriptor and what was learnt at the open. Hidden and changeable.
pub(crate) struct OpenFile {
    pub(crate) file: std::fs::File,
    pub(crate) path: PathBuf,
    /// The size at the open. A read checks its range against the file, not this number.
    pub(crate) size: u64,
    pub(crate) id: FileId,
    /// The backend that reads it.
    pub(crate) backend: Arc<dyn Backend>,
}

impl File {
    pub(crate) fn new(inner: Arc<OpenFile>) -> Self {
        Self { inner }
    }

    /// The path the file was opened by, as given to [`Io::open`](crate::Io::open).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// The file's size in bytes when it was opened.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.inner.size
    }

    /// The pool's number for this handle; see [`FileId`].
    #[must_use]
    pub fn id(&self) -> FileId {
        self.inner.id
    }

    /// Reads exactly `len` bytes at `offset`.
    ///
    /// Resolves to a buffer of exactly `len` bytes, allocated on the pool thread under
    /// `AllocTag::Io` and owned by the caller. `InvalidData` if the file ends before
    /// `offset + len` or the range overflows; `ResourceExhausted` if `len` exceeds
    /// [`MAX_READ_BYTES`](crate::MAX_READ_BYTES) or the buffer cannot be allocated, before
    /// any of it is; `Io` for anything else the read reports; `Cancelled` if the pool has
    /// stopped. A `len` of zero resolves to an empty buffer without touching the file.
    /// Nothing is submitted until the future is first polled.
    #[must_use = "a read does nothing until polled"]
    pub fn read_at(&self, offset: u64, len: u64) -> ReadAt {
        todo!("check the range, then a `Pending` of `Request::Read`; `Pending::failed` if not")
    }

    /// Reads the `len` stored bytes at `offset` and decodes them as `encoding`, as one
    /// request.
    ///
    /// Resolves to the decoded bytes: exactly `encoding`'s `decoded_len` of them, in a
    /// buffer allocated on the pool thread under `AllocTag::Io` and owned by the caller.
    /// The pool reads and checks the stored bytes; the decode runs in the returned future's
    /// own poll, on the thread that polls it (a background worker for SILO's loads,
    /// architecture section 5.1), in slices that yield at least every 0.5 ms. The stored
    /// bytes are never handed out. While it decodes, zstd holds a window buffer of its own,
    /// up to the smaller of the frame's window and `decoded_len` and at most 2^27 bytes,
    /// allocated outside the global allocator and so not charged to `AllocTag::Io`.
    /// `InvalidData` if the range is empty, short or overflows, if the stored bytes are not
    /// one whole frame of `encoding` ending exactly at `offset + len`, if the frame's window
    /// exceeds 2^27 bytes, or if they decode to any other length; `ResourceExhausted` if
    /// `len` or `decoded_len` exceeds [`MAX_READ_BYTES`](crate::MAX_READ_BYTES) or a buffer
    /// cannot be allocated, before either is, or if zstd cannot allocate its own memory;
    /// `Io` for anything else the read reports; `Cancelled` if the pool has stopped. Nothing
    /// is submitted until the future is first polled.
    #[must_use = "a read does nothing until polled"]
    pub fn read_encoded_at(&self, offset: u64, len: u64, encoding: Encoding) -> ReadAt {
        todo!(
            "check both lengths and the range, then a `Pending` of `Request::ReadEncoded`; \
             `Pending::failed` if not"
        )
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("File")
            .field("id", &self.inner.id)
            .field("path", &self.inner.path)
            .field("size", &self.inner.size)
            .finish_non_exhaustive()
    }
}

/// A positional read in flight: [`File::read_at`] or [`File::read_encoded_at`].
///
/// Resolves to the bytes asked for, every one of them or an error, never a short buffer.
/// Dropping it cancels the read: one not yet started never runs; one started finishes on
/// the pool thread and its buffer is freed there; an encoded read dropped after the pool
/// handed it its stored bytes frees both its buffers, and zstd's context, on the dropping
/// thread. The drop never blocks. The future holds the file open until the pool's part of
/// the read completes or the future is dropped.
///
/// Polling an encoded read whose stored bytes have arrived decodes a slice of them on the
/// polling thread, about 0.5 ms of work in steps of about 64 KiB of output, then wakes
/// itself and returns `Pending` until the frame is done, as section 5.1's background CPU
/// work yields.
#[must_use = "a read does nothing until polled"]
pub struct ReadAt {
    state: ReadState,
}

/// Where a read is. Hidden and changeable.
enum ReadState {
    /// A raw read, or one failed before it was made.
    Raw(Pending<Vec<u8>>),
    /// An encoded read whose stored range the pool has not yet returned.
    Stored(Pending<Encoded>),
    /// An encoded read decoding in this future's polls.
    Decoding(Decode),
    /// Resolved; polling again is a broken invariant.
    Done,
}

impl Future for ReadAt {
    type Output = Result<Vec<u8>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Vec<u8>>> {
        todo!(
            "Raw: the pending's result. Stored: on `Encoded::Stored` move to `Decoding` with \
             the pending's span and decode a first slice; `Encoded::Decoded` is the result. \
             Decoding: `Decode::poll_slice`"
        )
    }
}
