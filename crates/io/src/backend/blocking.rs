//! The blocking backend: a FIFO queue, `pool_threads` platform threads of class
//! `ThreadClass::Io`, and each request as the system calls one of them makes for it.
//!
//! The functions after [`BlockingPool`] run on a pool thread, inside the request's span
//! and under `AllocTag::Io`, and return what the request's future resolves to. The
//! contract of each is the public method's documentation and `README.md`; the comments
//! below are the steps that meet it, for the implementation to follow or improve on.

use core::num::NonZero;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use lactose_core::Result;
use lactose_task::{Platform, ThreadHandle};

use crate::backend::Backend;
use crate::file::{FileId, OpenFile};
use crate::request::Request;
use crate::types::{DirEntry, Encoding};

/// The reference backend.
pub(crate) struct BlockingPool {
    /// Requests waiting for a thread, in submission order, and whether the pool is
    /// stopping.
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
    /// stops. A cancelled request is skipped; a stopping pool fails what is still queued
    /// instead of running it.
    fn run(self: Arc<Self>) {
        todo!("wait for a request or stopping; run or cancel it; until stopping and empty")
    }

    /// Runs one request on this thread, inside its span, and completes it.
    fn run_one(&self, request: Request) {
        todo!("match the variant to the function below and complete")
    }
}

impl Backend for BlockingPool {
    fn submit(&self, request: Request) {
        todo!("push under the lock and notify one, or cancel the request if stopping")
    }

    fn stop(&self) {
        todo!("set stopping, cancel every queued request, notify all")
    }

    fn join(&self) {
        todo!("stop, then take and join every thread handle")
    }
}

/// Opens `path` for reading and records its size. `NotFound` if there is no such file;
/// `Io` for a directory, a permission failure or anything else `std::fs::File::open`
/// reports.
fn open(backend: Arc<dyn Backend>, path: &Path, id: FileId) -> Result<Arc<OpenFile>> {
    todo!("File::open, metadata().len(), an OpenFile holding the backend for its reads")
}

/// Exactly `len` bytes at `offset` into a fresh buffer: `read_exact_at` on Unix,
/// `seek_read` in a loop on Windows (which may read short). `UnexpectedEof` maps to
/// `InvalidData` through `From<std::io::Error>`; the buffer is dropped with it, so no
/// partial buffer is handed back.
fn read_at(file: &OpenFile, offset: u64, len: u64) -> Result<Vec<u8>> {
    todo!("one allocation of `len`, filled completely or dropped")
}

/// [`read_at`] followed by one decode of the stored bytes as `encoding` into a second
/// buffer of exactly `decoded_len` bytes, both allocations under `AllocTag::Io`. One
/// frame, consuming every stored byte and producing exactly `decoded_len`; anything
/// else is `InvalidData`. The stored buffer is freed here.
fn read_encoded_at(file: &OpenFile, offset: u64, len: u64, encoding: Encoding) -> Result<Vec<u8>> {
    todo!(
        "read_at, then zstd::bulk::Decompressor::decompress_to_buffer into a buffer of decoded_len, checking the count"
    )
}

/// The whole file at `path`, read to its end: `NotFound`, `Io` as for [`open`];
/// `ResourceExhausted` if its size exceeds `MAX_READ_BYTES` before anything is allocated.
fn read_file(path: &Path) -> Result<Vec<u8>> {
    todo!("open, check the size against the cap, read_to_end into a buffer of that capacity")
}

/// Architecture section 15.3's atomic replace, as one request:
///
/// 1. Create `<name>.<unique>.tmp` in `path`'s directory with `create_new`, so a leftover
///    of another attempt is never reused; `NotFound` if the directory does not exist.
/// 2. `write_all(bytes)`, then `sync_all` (`fsync`; `F_FULLFSYNC` on Apple platforms,
///    which `std` issues for `sync_all`; `FlushFileBuffers` on Windows). `StorageFull`
///    from either step maps through `From<std::io::Error>`.
/// 3. `rename` the temporary over `path`. Same directory, so never across volumes.
/// 4. On Unix, open the directory and `sync_all` it, so the rename is durable too.
///
/// Any failure removes the temporary file (best effort) and leaves `path` as it was. A
/// started replace runs all four steps however its future was dropped.
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    todo!("the four steps above, with the temporary removed on any error")
}

/// Every entry of the directory at `path`, by its own file type (a symbolic link is
/// `Other`), sorted by name. `NotFound` if there is no such directory; `Io` if `path` is
/// not a directory or cannot be read. A `.tmp` left by a crashed [`replace`] is listed
/// like any other file; callers that list slots ignore the suffix.
fn read_dir(path: &Path) -> Result<Vec<DirEntry>> {
    todo!("read_dir, file_type() per entry, collect, sort_unstable by name")
}

/// `std::fs::create_dir_all`: every missing directory of `path`, and success if all of
/// them exist already. `Io` on a permission failure or a file in the way.
fn create_dir_all(path: &Path) -> Result<()> {
    todo!("create_dir_all")
}
