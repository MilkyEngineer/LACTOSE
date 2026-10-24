//! The blocking backend: a FIFO queue, `pool_threads` platform threads of class
//! `ThreadClass::Io`, and each request as the system calls one of them makes for it.
//!
//! The functions after [`BlockingPool`] run on a pool thread, inside the request's span
//! and under `AllocTag::Io`, and return what the request's future resolves to. The
//! contract of each is the public method's documentation and `README.md`; the comments
//! below are the steps that meet it, for the implementation to follow or improve on.
//!
//! Every `std::io::Error` becomes a `lactose_core::Error` through [`os_error`], never
//! through `From` alone: a request returns only the kinds `README.md` lists.

use core::num::NonZero;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use lactose_core::{Error, Result};
use lactose_task::{Platform, ThreadHandle};

use crate::backend::Backend;
use crate::file::{FileId, OpenFile};
use crate::request::{Encoded, Request};
use crate::types::{DirEntry, Encoding, ReplaceFailed};

/// The reference backend.
pub(crate) struct BlockingPool {
    /// Requests waiting for a thread, in submission order, the replaces waiting behind
    /// another of their path, and whether the pool is stopping.
    queue: Mutex<Queue>,
    /// Signalled when a request is queued or the pool stops.
    available: Condvar,
    /// The threads, until `join` takes them.
    threads: Mutex<Vec<ThreadHandle>>,
    /// The configured thread count.
    pool_threads: NonZero<u32>,
}

struct Queue {
    requests: VecDeque<Request>,
    /// Replaces of one path run in submission order, dropped or not, through this
    /// waitlist and never by a pool thread waiting. A path is a key from the submission of
    /// its first replace until the last one queued behind it has finished: a replace of a
    /// path with a key waits here, in order, instead of entering `requests`, and the thread
    /// that finishes one, or skips one whose future was dropped before it started, moves
    /// the next to the back of `requests` (dropping, unrun, any whose future was dropped
    /// while it waited), removing the key when none is left.
    /// Keys compare as `Path` values, so two spellings of one file are two keys.
    replacing: HashMap<PathBuf, VecDeque<Request>>,
    stopping: bool,
}

impl BlockingPool {
    /// Starts `pool_threads` threads through `platform.spawn_thread`, each
    /// `ThreadSpec { name: "io-<index>", class: ThreadClass::Io, index_in_class: index }`,
    /// each running [`BlockingPool::run`] under `AllocTag::Io`. Fails with what the
    /// platform reports if a thread cannot be created, after stopping the ones created.
    pub(crate) fn start(platform: &dyn Platform, pool_threads: NonZero<u32>) -> Result<Arc<Self>> {
        todo!("spawn the threads, each holding an Arc of the pool")
    }

    /// A pool thread's life: take the next request in order and run it, until the pool
    /// stops. A cancelled request is skipped; a stopping pool fails what is still queued,
    /// waitlists included, instead of running it.
    fn run(self: Arc<Self>) {
        todo!("wait for a request or stopping; run or cancel it; until stopping and empty")
    }

    /// Runs one request on this thread, inside its span, and completes it; for a replace,
    /// then releases the next replace of its path from the waitlist.
    fn run_one(&self, request: Request) {
        todo!("match the variant to the function below and complete")
    }
}

impl Backend for BlockingPool {
    fn submit(&self, request: Request) {
        todo!(
            "under the lock: cancel the request if stopping; a replace whose path has a key \
             joins its waitlist, one whose path has none takes a key and is queued; push and \
             notify one"
        )
    }

    fn stop(&self) {
        todo!("set stopping, cancel every queued and waitlisted request, notify all")
    }

    fn join(&self) {
        todo!("stop, then take and join every thread handle")
    }
}

/// A system call's error as a request's: `From<std::io::Error>`, keeping the source, with
/// the kinds no request returns made `Io`. `Unsupported` arises from `ENOTSUP` (an
/// `F_FULLFSYNC` on a file system without it) and `Network` from a timeout or a reset on a
/// network file system; neither is a policy `app` has for a file.
fn os_error(error: std::io::Error) -> Error {
    todo!("Error::from(error), its kind remapped from Unsupported or Network to Io")
}

/// Opens `path` for reading and records its size. `NotFound` if there is no such file;
/// `Io` for a directory, a permission failure or anything else `std::fs::File::open`
/// reports.
fn open(backend: Arc<dyn Backend>, path: &Path, id: FileId) -> Result<Arc<OpenFile>> {
    todo!("File::open, metadata().len(), an OpenFile holding the backend for its reads")
}

/// Exactly `len` bytes at `offset` into a fresh buffer:
///
/// 1. `metadata().len()` of the open file (fstat): a range past it is `InvalidData`
///    before anything is allocated.
/// 2. `Vec::try_reserve_exact(len)`: a failure is `ResourceExhausted`, never an abort.
/// 3. `read_exact_at` on Unix, `seek_read` in a loop on Windows (which may read short). A
///    file truncated since step 1 gives `UnexpectedEof`, which is `InvalidData`; the
///    buffer is dropped with it, so no partial buffer is handed back.
///
/// On Windows `std`'s handle is synchronous and the system serialises every operation on
/// it, so reads on one `File` run one at a time whatever `pool_threads` is
/// (`docs/verification.md` has the row).
fn read_at(file: &OpenFile, offset: u64, len: u64) -> Result<Vec<u8>> {
    todo!("fstat, one fallible allocation of `len`, filled completely or dropped")
}

/// The pool's part of an encoded read: the stored frame read and checked, and the decoded
/// buffer reserved, for the future to decode in its own poll ([`Encoded::Stored`]):
///
/// 1. [`read_at`] of the stored range (fstat first, a fallible allocation of `len`).
/// 2. Before anything else is allocated, every one of these, each failure `InvalidData`:
///    the first four bytes are zstd's frame magic (`0xFD2FB528`, little-endian), so the
///    frame is not a skippable one; `zstd_safe::find_frame_compressed_size(stored)` is
///    `Ok(len)`, so the range is exactly one frame, not truncated, with nothing after it
///    (a second, concatenated frame included); `zstd_safe::get_frame_content_size(stored)`
///    is `Ok(Some(decoded_len))`.
/// 3. `Vec::try_reserve_exact(decoded_len)` for the output: a failure is
///    `ResourceExhausted`.
///
/// Every zstd error maps to `InvalidData` explicitly, from its code and
/// `zstd_safe::get_error_name`, never through `From<std::io::Error>`, which made them
/// `Io`. The decoded count is checked by the decode (`decode.rs`), which also maps its
/// stream's errors to `InvalidData`.
fn read_encoded_at(file: &OpenFile, offset: u64, len: u64, encoding: Encoding) -> Result<Encoded> {
    todo!("read_at, the three frame checks, then the reserved output buffer")
}

/// The whole file at `path`: open it, take its size from `metadata()` (fstat),
/// `ResourceExhausted` if it exceeds `MAX_READ_BYTES` or `try_reserve_exact` of it fails,
/// before anything is read; then read exactly that many bytes (`read_exact`: a file that
/// shrank is `UnexpectedEof`, so `InvalidData`) and try to read one more. A byte read
/// there means the file grew while it was read, and the request is `InvalidData` with
/// the buffer dropped, never a buffer past the cap and never a silent truncation. A
/// caller that reads a file being written retries.
fn read_file(path: &Path) -> Result<Vec<u8>> {
    todo!("open, fstat, cap, fallible allocation, read_exact, a one-byte probe for growth")
}

/// Architecture section 15.3's atomic replace, as one request, with the bytes handed back
/// in the error on every failure:
///
/// 0. The directory is `path.parent()`, and `"."` when that is empty (`slot0.carton`). A
///    `path` with no file name is `Io` before anything is touched.
/// 1. Sweep: list the directory and remove every `<name>.<unique>.tmp` whose `<unique>`
///    has this crate's form, sixteen lowercase hexadecimal digits, so another path's
///    temporary never matches (`<name>` is `path`'s file name; a sibling `a.b` has
///    temporaries `a.b.<unique>.tmp`, whose middle is not sixteen hex digits for `a`).
///    Best effort: a listing or removal that fails is ignored, and the create below
///    reports the directory's real state. Safe because replaces of one path are ordered
///    within the process (the waitlist); two processes replacing one path are unsupported.
/// 2. Create `<name>.<unique>.tmp` in the directory with `create_new`, so a leftover is
///    never reused; `<unique>` is the process id and a counter, each as eight hex digits.
///    On `AlreadyExists` take the next counter value and try again, up to 16 times, then
///    `Io`. `NotFound` if the directory does not exist.
/// 3. `write_all(bytes)`, then `sync_all` (`fsync`; `F_FULLFSYNC` on Apple platforms,
///    which `std` issues for `sync_all`; `FlushFileBuffers` on Windows). `StorageFull`
///    from either step.
/// 4. `rename` the temporary over `path`. Same directory, so never across volumes.
/// 5. On Unix, open the directory and `sync_all` it, so the rename is durable too.
///
/// A failure in steps 2 to 4 removes the temporary file (best effort) and leaves `path` as
/// it was, with the kind [`os_error`] gives. A failure in step 5 is `Io` whatever the
/// system call said (a directory removed after the rename would otherwise be `NotFound`,
/// which `app` reads as "no save directory"): the new content is in place and its
/// durability unknown. A started replace runs every step however its future was dropped.
fn replace(path: &Path, bytes: Vec<u8>) -> Result<(), ReplaceFailed> {
    todo!("the steps above, `bytes` returned in the error on every failure")
}

/// Every entry of the directory at `path`, by its own file type (a symbolic link is
/// `Other`), sorted by name. `NotFound` if there is no such directory; `Io` if `path` is
/// not a directory or cannot be read. A `.tmp` left by a crashed [`replace`] is listed
/// like any other file until the next replace of its path removes it; callers that list
/// slots ignore the suffix.
fn read_dir(path: &Path) -> Result<Vec<DirEntry>> {
    todo!("read_dir, file_type() per entry, collect, sort_unstable by name")
}

/// `std::fs::create_dir_all`: every missing directory of `path`, and success if all of
/// them exist already. `Io` on a permission failure or a file in the way, and for a
/// `NotFound` (a directory on the way removed while it ran); `StorageFull` for a full disk.
fn create_dir_all(path: &Path) -> Result<()> {
    todo!("create_dir_all, a NotFound made Io")
}
