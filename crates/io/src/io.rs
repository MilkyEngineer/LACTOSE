//! The pool: its handle, its start and stop, and the requests that take a path.

use core::num::NonZero;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use lactose_core::Result;
use lactose_task::Platform;

use crate::backend::Backend;
use crate::backend::blocking::BlockingPool;
use crate::config::IoConfig;
use crate::file::{File, FileId};
use crate::request::{Completion, Failure, Pending, Request};
use crate::types::{DirEntry, ReplaceFailed};

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
        let backend = BlockingPool::start(platform, config.pool_threads)?;
        Ok(Self::over(backend, config.pool_threads))
    }

    /// [`Io::start`], with the crate's tests told of every point of the pool's work.
    #[cfg(test)]
    pub(crate) fn start_hooked(
        platform: &dyn Platform,
        config: IoConfig,
        hook: crate::backend::blocking::Hook,
    ) -> Result<Self> {
        let backend = BlockingPool::start_hooked(platform, config.pool_threads, hook)?;
        Ok(Self::over(backend, config.pool_threads))
    }

    fn over(backend: Arc<dyn Backend>, pool_threads: NonZero<u32>) -> Self {
        Self { owner: Arc::new(Owner { backend, next_file_id: AtomicU64::new(0), pool_threads }) }
    }

    /// A request for the backend: its completion, and the future's side of it.
    fn pending<T, E: Failure>(
        &self,
        request: impl FnOnce(Arc<Completion<T, E>>) -> Request,
    ) -> Pending<T, E> {
        let completion = Completion::new();
        let request = request(Arc::clone(&completion));
        Pending::new(Arc::clone(&self.owner.backend), completion, request)
    }

    /// The number of threads the pool was started with, which is also the number of
    /// requests it runs at once.
    #[must_use]
    pub fn pool_threads(&self) -> NonZero<u32> {
        self.owner.pool_threads
    }

    /// Opens the file at `path` for reading.
    ///
    /// Resolves to a [`File`] that knows its size. The handle's [`FileId`](crate::FileId)
    /// is taken now, not at the first poll. `NotFound` if there is no such file; `Io` for a
    /// directory, a permission failure or anything else the open reports; `Cancelled` if
    /// the pool has stopped.
    #[must_use = "an open does nothing until polled"]
    pub fn open(&self, path: impl Into<PathBuf>) -> Open {
        let id = FileId(self.owner.next_file_id.fetch_add(1, Ordering::Relaxed));
        let path = path.into();
        Open { pending: self.pending(|complete| Request::Open { path, id, complete }) }
    }

    /// Reads the whole file at `path`.
    ///
    /// Resolves to its content, in a buffer allocated on the pool thread under
    /// `AllocTag::Io` and owned by the caller. `NotFound` and `Io` as for [`Io::open`];
    /// `ResourceExhausted` if the file is larger than [`MAX_READ_BYTES`](crate::MAX_READ_BYTES)
    /// or its buffer cannot be allocated, before anything is; `InvalidData` if the file's
    /// size changes while it is read; `Cancelled` if the pool has stopped. For a file read
    /// in ranges, [`Io::open`] and [`File::read_at`].
    #[must_use = "a read does nothing until polled"]
    pub fn read_file(&self, path: impl Into<PathBuf>) -> ReadFile {
        let path = path.into();
        ReadFile { pending: self.pending(|complete| Request::ReadFile { path, complete }) }
    }

    /// Replaces the content of the file at `path` with `bytes`, atomically and durably:
    /// architecture section 15.3's temporary file, sync and rename as one request.
    ///
    /// When the future resolves `Ok`, a reader of `path` on this machine sees all of
    /// `bytes`, and a crash or power loss after that point keeps them. At no point does a
    /// reader see anything but the previous content or the new. The temporary file is
    /// `<name>.<unique>.tmp` in the same directory as `path`, never on another volume; a
    /// `path` with no directory part is in the current directory. The first replace into a
    /// directory removes the temporaries an earlier process left there, never one of this
    /// process. Replaces of one `path`, spelled the same, run in the order they were
    /// submitted, dropped or not, so the last one submitted lands last; under two spellings
    /// or from two `Io`s they are unordered, the last rename wins, and neither fails because
    /// of the other. Two processes writing into one directory are unsupported. A `path`
    /// whose last component is a symbolic link becomes a regular file, the link's target
    /// unchanged.
    ///
    /// On failure the future hands `bytes` back, unchanged, in a [`ReplaceFailed`] beside
    /// the error. A failure before the rename keeps `path`'s previous content and removes
    /// the temporary: `StorageFull` if the disk or quota fills while writing or syncing;
    /// `NotFound` if `path`'s directory does not exist (callers create it with
    /// [`Io::create_dir_all`] first); `Io` for a permission failure, a `path` that is a
    /// directory or names no file, or on Windows a `path` another process holds open
    /// without delete sharing. A failure syncing the directory after the rename is `Io`,
    /// with the new content in place and its durability unknown; every failure after the
    /// rename is `Io`. `Cancelled` if the pool has stopped.
    #[must_use = "a replace does nothing until polled"]
    pub fn replace(&self, path: impl Into<PathBuf>, bytes: Vec<u8>) -> Replace {
        let path = path.into();
        Replace { pending: self.pending(|complete| Request::Replace { path, bytes, complete }) }
    }

    /// Lists the directory at `path`.
    ///
    /// Resolves to every entry, sorted by name, each with its own kind (a symbolic link is
    /// `EntryKind::Other`, whatever it points to). `NotFound` if there is no such
    /// directory; `Io` if `path` is not a directory or cannot be read; `Cancelled` if the
    /// pool has stopped.
    #[must_use = "a listing does nothing until polled"]
    pub fn read_dir(&self, path: impl Into<PathBuf>) -> ReadDir {
        let path = path.into();
        ReadDir { pending: self.pending(|complete| Request::ReadDir { path, complete }) }
    }

    /// Creates the directory at `path` and every missing parent; succeeds if it exists.
    ///
    /// `Io` on a permission failure or when a file is in the way; `StorageFull` if the disk
    /// or quota is full; `Cancelled` if the pool has stopped.
    #[must_use = "a create does nothing until polled"]
    pub fn create_dir_all(&self, path: impl Into<PathBuf>) -> CreateDirAll {
        let path = path.into();
        CreateDirAll { pending: self.pending(|complete| Request::CreateDirAll { path, complete }) }
    }

    /// Stops the pool and waits for its threads.
    ///
    /// Requests already started finish; requests queued and requests submitted after this
    /// call, from any clone, resolve `Cancelled`. Blocks the calling thread until every
    /// thread has exited, so it is for the main thread and tools, before the executor's
    /// own shutdown. Clones left after this call are inert handles.
    ///
    /// Never call it from a pool thread, which runs the wakers of the requests it
    /// completes: behind a `shutdown` already joining that thread it hangs, and alone it
    /// joins the thread it runs on, which panics (and so aborts a binary).
    pub fn shutdown(self) {
        self.owner.backend.join();
    }
}

impl fmt::Debug for Io {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Io").field("pool_threads", &self.pool_threads()).finish_non_exhaustive()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.backend.stop();
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

/// An atomic replace in flight: [`Io::replace`]. Resolves once the new content is durable,
/// or to a [`ReplaceFailed`] holding the bytes. Dropping it does not tear the file: a
/// replace not yet started never runs, and one started runs to its end, so the path holds
/// the previous content or the new, never a part of either; a replace of the same path
/// submitted after it still lands after it.
#[must_use = "a replace does nothing until polled"]
pub struct Replace {
    pending: Pending<(), ReplaceFailed>,
}

impl Future for Replace {
    type Output = Result<(), ReplaceFailed>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ReplaceFailed>> {
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
