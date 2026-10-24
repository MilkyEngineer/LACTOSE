//! The pool: its handle, its start and stop, and the requests that take a path.

use core::num::NonZero;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{Context, Poll};

use lactose_core::Result;
use lactose_task::Platform;

use crate::backend::Backend;
use crate::config::IoConfig;
use crate::file::File;
use crate::request::Pending;
use crate::types::DirEntry;

/// A handle to the file IO pool: `pool_threads` platform threads of class `ThreadClass::Io`
/// that complete every request of this crate.
///
/// One per process, started by `app` from the executor's platform and cloned into
/// whatever submits requests: SILO's store, the save path, the cooker. Every method that
/// returns a future submits nothing until that future is first polled; the future is
/// `Send + 'static`, so it is awaited in place, spawned on the executor, or driven by
/// `Executor::block_on` from the main thread or a tool.
///
/// Dropping the last clone stops the pool: queued requests resolve `Cancelled`, started
/// ones finish, the threads exit on their own and are not joined. [`Io::shutdown`] is the
/// orderly form.
#[derive(Clone)]
pub struct Io {
    owner: Arc<Owner>,
}

/// What the handles own and the backend's threads do not: dropping the last handle stops
/// the backend. Hidden and changeable.
struct Owner {
    backend: Arc<dyn Backend>,
    /// The next [`FileId`](crate::FileId).
    next_file_id: AtomicU64,
    /// The configured thread count.
    pool_threads: NonZero<u32>,
}

impl Io {
    /// Starts the pool: `config.pool_threads` threads through `platform.spawn_thread`,
    /// each `ThreadSpec { name: "io-<index>", class: ThreadClass::Io, index_in_class }`,
    /// each under `AllocTag::Io` for its whole life.
    ///
    /// Fails with what the platform reports if a thread cannot be created; the threads
    /// already created are stopped first. Nothing of the executor is kept: the platform is
    /// used for the spawns and released.
    pub fn start(platform: &dyn Platform, config: IoConfig) -> Result<Self> {
        todo!("BlockingPool::start(platform, config.pool_threads) behind an Owner")
    }

    /// The number of threads the pool was started with, which is also the number of
    /// requests it runs at once.
    #[must_use]
    pub fn pool_threads(&self) -> NonZero<u32> {
        self.owner.pool_threads
    }

    /// Opens the file at `path` for reading.
    ///
    /// Resolves to a [`File`] that knows its size. `NotFound` if there is no such file;
    /// `Io` for a directory, a permission failure or anything else the open reports;
    /// `Cancelled` if the pool has stopped.
    #[must_use = "an open does nothing until polled"]
    pub fn open(&self, path: impl Into<PathBuf>) -> Open {
        todo!("a `Pending` of `Request::Open` with the next FileId under `span::open`")
    }

    /// Reads the whole file at `path`.
    ///
    /// Resolves to its content, in a buffer allocated on the pool thread under
    /// `AllocTag::Io` and owned by the caller. `NotFound` and `Io` as for [`Io::open`];
    /// `ResourceExhausted` if the file is larger than [`MAX_READ_BYTES`](crate::MAX_READ_BYTES),
    /// before anything is allocated; `Cancelled` if the pool has stopped. For a file read
    /// in ranges, [`Io::open`] and [`File::read_at`].
    #[must_use = "a read does nothing until polled"]
    pub fn read_file(&self, path: impl Into<PathBuf>) -> ReadFile {
        todo!("a `Pending` of `Request::ReadFile` under `span::read_file`")
    }

    /// Replaces the content of the file at `path` with `bytes`, atomically and durably:
    /// architecture section 15.3's temporary file, sync and rename as one request.
    ///
    /// When the future resolves `Ok`, a reader of `path` on this machine sees all of
    /// `bytes`, and a crash or power loss after that point keeps them. At no point does a
    /// reader see anything but the previous content or the new. The temporary file is
    /// `<name>.<unique>.tmp` in the same directory as `path`, never on another volume.
    /// `bytes` is consumed; a caller that may retry keeps its own copy.
    ///
    /// `StorageFull` if the disk or quota fills while writing or syncing; `NotFound` if
    /// `path`'s directory does not exist (callers create it with [`Io::create_dir_all`]
    /// first); `Io` for a permission failure, a `path` that is a directory, or on Windows
    /// a `path` another process holds open without delete sharing; `Cancelled` if the
    /// pool has stopped. On any failure `path` keeps its previous content and the
    /// temporary file is removed.
    #[must_use = "a replace does nothing until polled"]
    pub fn replace(&self, path: impl Into<PathBuf>, bytes: Vec<u8>) -> Replace {
        todo!("a `Pending` of `Request::Replace` under `span::replace`")
    }

    /// Lists the directory at `path`.
    ///
    /// Resolves to every entry, sorted by name, each with its own kind (a symbolic link is
    /// `EntryKind::Other`, whatever it points to). `NotFound` if there is no such
    /// directory; `Io` if `path` is not a directory or cannot be read; `Cancelled` if the
    /// pool has stopped.
    #[must_use = "a listing does nothing until polled"]
    pub fn read_dir(&self, path: impl Into<PathBuf>) -> ReadDir {
        todo!("a `Pending` of `Request::ReadDir` under `span::read_dir`")
    }

    /// Creates the directory at `path` and every missing parent; succeeds if it exists.
    ///
    /// `Io` on a permission failure or when a file is in the way; `Cancelled` if the pool
    /// has stopped.
    #[must_use = "a create does nothing until polled"]
    pub fn create_dir_all(&self, path: impl Into<PathBuf>) -> CreateDirAll {
        todo!("a `Pending` of `Request::CreateDirAll` under `span::create_dir_all`")
    }

    /// Stops the pool and waits for its threads.
    ///
    /// Requests already started finish; requests queued and requests submitted after this
    /// call, from any clone, resolve `Cancelled`. Blocks the calling thread until every
    /// thread has exited, so it is for the main thread and tools, before the executor's
    /// own shutdown. Clones left after this call are inert handles.
    pub fn shutdown(self) {
        todo!("backend.stop(); backend.join()")
    }
}

impl fmt::Debug for Io {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Io").field("pool_threads", &self.pool_threads()).finish_non_exhaustive()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        todo!("backend.stop(); no join")
    }
}

/// An open in flight: [`Io::open`]. Resolves to a [`File`]. Dropping it cancels the open;
/// a descriptor opened before the drop is closed on the pool thread.
#[must_use = "an open does nothing until polled"]
pub struct Open {
    pending: Pending<Arc<crate::file::OpenFile>>,
}

impl Future for Open {
    type Output = Result<File>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<File>> {
        Pin::new(&mut self.pending).poll(cx).map(|result| result.map(File::new))
    }
}

/// A whole-file read in flight: [`Io::read_file`]. Resolves to the file's content.
/// Dropping it cancels the read; a buffer allocated before the drop is freed on the pool
/// thread.
#[must_use = "a read does nothing until polled"]
pub struct ReadFile {
    pending: Pending<Vec<u8>>,
}

impl Future for ReadFile {
    type Output = Result<Vec<u8>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Vec<u8>>> {
        Pin::new(&mut self.pending).poll(cx)
    }
}

/// An atomic replace in flight: [`Io::replace`]. Resolves once the new content is durable.
/// Dropping it does not tear the file: a replace not yet started never runs, and one
/// started runs to its end, so the path holds the previous content or the new, never a
/// part of either.
#[must_use = "a replace does nothing until polled"]
pub struct Replace {
    pending: Pending<()>,
}

impl Future for Replace {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.pending).poll(cx)
    }
}

/// A directory listing in flight: [`Io::read_dir`]. Resolves to the entries sorted by
/// name.
#[must_use = "a listing does nothing until polled"]
pub struct ReadDir {
    pending: Pending<Vec<DirEntry>>,
}

impl Future for ReadDir {
    type Output = Result<Vec<DirEntry>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Vec<DirEntry>>> {
        Pin::new(&mut self.pending).poll(cx)
    }
}

/// A directory creation in flight: [`Io::create_dir_all`].
#[must_use = "a create does nothing until polled"]
pub struct CreateDirAll {
    pending: Pending<()>,
}

impl Future for CreateDirAll {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.pending).poll(cx)
    }
}
