//! The blocking backend: a FIFO queue, `pool_threads` platform threads of class
//! `ThreadClass::Io`, and each request as the system calls one of them makes for it.
//!
//! The functions after [`BlockingPool`] run on a pool thread, inside the request's span
//! and under `AllocTag::Io`, and return what the request's future resolves to. The
//! contract of each is the public method's documentation and `README.md`.
//!
//! Every `std::io::Error` becomes a `lactose_core::Error` through [`os_error`], never
//! through `From` alone: a request returns only the kinds `README.md` lists.

use core::num::NonZero;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::hash::BuildHasher;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};

use lactose_core::{AllocTag, Error, ErrorKind, Result, ThreadClass};
use lactose_task::{Platform, ThreadHandle, ThreadSpec};

use crate::backend::Backend;
use crate::decode;
use crate::file::{FileId, OpenFile};
use crate::request::{Encoded, Request};
use crate::types::{DirEntry, Encoding, EntryKind, MAX_READ_BYTES, ReplaceFailed};

/// The reference backend.
pub(crate) struct BlockingPool {
    /// Requests waiting for a thread, in submission order, the replaces waiting behind
    /// another of their path, and whether the pool is stopping.
    queue: Mutex<Queue>,
    /// Signalled when a request is queued or the pool stops.
    available: Condvar,
    /// The threads, until `join` takes them.
    threads: Mutex<Vec<ThreadHandle>>,
    /// The directories [`replace`] has swept for this pool, keyed as spelled: the first
    /// replace into a directory takes its key and sweeps it, every later one skips the
    /// sweep. A directory spelled two ways is swept twice, which costs one more listing.
    /// Held only to test and insert, never across the sweep.
    swept: Mutex<HashSet<PathBuf>>,
    /// What the crate's own tests observe of the pool's work.
    #[cfg(test)]
    hook: Option<Hook>,
}

/// A point in a request's life that the crate's tests observe, and may hold a pool thread
/// at, to make an ordering deterministic.
#[cfg(test)]
pub(crate) enum Point<'a> {
    /// A request handed to `submit`, before the queue sees it, on the submitting thread.
    Submitted(&'a Request),
    /// A request about to run on this pool thread, inside its span.
    Started(&'a Request),
    /// A replace's temporary, just created, before anything is written to it.
    Created(&'a Path),
    /// A replace's temporary, written and just synced, before the rename; then its
    /// directory, just synced after the rename (on Windows, which has no directory sync,
    /// just after the rename). Told by the sync functions themselves, so a replace that
    /// skips a sync tells nothing for it.
    Synced(&'a Path),
}

/// The observer of [`Point`]s a test starts a pool with.
#[cfg(test)]
pub(crate) type Hook = Arc<dyn Fn(Point<'_>) + Send + Sync>;

/// How many unique temporary names a replace tries before it gives up with `Io`.
const CREATE_ATTEMPTS: u32 = 16;

/// This process's temporary-name token: eight lower-case hex digits, taken once per
/// process from `std::hash::RandomState` (randomly keyed per process) hashing the process
/// id and the time of the first call. A temporary carries it, so a sweep tells this
/// process's live temporaries from what another process left.
pub(crate) fn token() -> u32 {
    static TOKEN: OnceLock<u32> = OnceLock::new();
    *TOKEN.get_or_init(|| {
        let seed = (std::process::id(), std::time::SystemTime::now());
        // Truncation is the intent: eight hex digits of a randomly keyed hash.
        std::hash::RandomState::new().hash_one(seed) as u32
    })
}

/// The process-wide counter behind a temporary's last eight hex digits.
pub(crate) static COUNTER: AtomicU32 = AtomicU32::new(0);

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
        Self::spawn(Arc::new(Self::new()), platform, pool_threads)
    }

    /// [`BlockingPool::start`], with `hook` told of every [`Point`].
    #[cfg(test)]
    pub(crate) fn start_hooked(
        platform: &dyn Platform,
        pool_threads: NonZero<u32>,
        hook: Hook,
    ) -> Result<Arc<Self>> {
        let mut pool = Self::new();
        pool.hook = Some(hook);
        Self::spawn(Arc::new(pool), platform, pool_threads)
    }

    fn new() -> Self {
        Self {
            queue: Mutex::new(Queue {
                requests: VecDeque::new(),
                replacing: HashMap::new(),
                stopping: false,
            }),
            available: Condvar::new(),
            threads: Mutex::new(Vec::new()),
            swept: Mutex::new(HashSet::new()),
            #[cfg(test)]
            hook: None,
        }
    }

    fn spawn(
        pool: Arc<Self>,
        platform: &dyn Platform,
        pool_threads: NonZero<u32>,
    ) -> Result<Arc<Self>> {
        for index in 0..pool_threads.get() {
            let spec = ThreadSpec {
                name: format!("io-{index}"),
                class: ThreadClass::Io,
                index_in_class: index,
            };
            let worker = Arc::clone(&pool);
            let body = Box::new(move || {
                let _tag = AllocTag::Io.scope();
                worker.run();
            });
            match platform.spawn_thread(spec, body) {
                Ok(thread) => lock(&pool.threads).push(thread),
                Err(error) => {
                    pool.join();
                    return Err(error);
                }
            }
        }
        Ok(pool)
    }

    /// A pool thread's life: take the next request in order and run it, until the pool
    /// stops. A cancelled request is skipped; a stopping pool fails what is still queued,
    /// waitlists included, instead of running it (in [`Backend::stop`]).
    fn run(self: Arc<Self>) {
        let backend: Arc<dyn Backend> = self.clone();
        while let Some(request) = self.next() {
            self.run_one(&backend, request);
        }
    }

    /// The next request in submission order, waiting for one; `None` once the pool stops.
    fn next(&self) -> Option<Request> {
        let mut queue = lock(&self.queue);
        loop {
            if let Some(request) = queue.requests.pop_front() {
                return Some(request);
            }
            if queue.stopping {
                return None;
            }
            queue = self.available.wait(queue).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Runs one request on this thread, inside its span, and completes it; for a replace,
    /// then releases the next replace of its path from the waitlist.
    fn run_one(&self, backend: &Arc<dyn Backend>, request: Request) {
        if request.is_cancelled() {
            let path = request.replace_path().map(Path::to_path_buf);
            drop(request);
            if let Some(path) = path {
                self.release(&path);
            }
            return;
        }
        let span = request.current_span();
        let _entered = span.enter();
        #[cfg(test)]
        self.observe(Point::Started(&request));
        match request {
            Request::Open { path, id, complete } => {
                let result = open(backend, path, id);
                if let Ok(file) = &result {
                    span.record("size", file.size);
                }
                complete.complete(result);
            }
            Request::Read { file, offset, len, complete } => {
                complete.complete(read_at(&file, offset, len));
            }
            Request::ReadEncoded { file, offset, len, encoding, complete } => {
                complete.complete(read_encoded_at(&file, offset, len, encoding));
            }
            Request::ReadFile { path, complete } => {
                let result = read_file(&path);
                if let Ok(bytes) = &result {
                    span.record("len", bytes.len() as u64);
                }
                complete.complete(result);
            }
            Request::Replace { path, bytes, complete } => {
                #[cfg(test)]
                let (created, synced) = (
                    |temporary: &Path| self.observe(Point::Created(temporary)),
                    |synced: &Path| self.observe(Point::Synced(synced)),
                );
                #[cfg(not(test))]
                let (created, synced) = (|_: &Path| {}, |_: &Path| {});
                let steps = Steps { created: &created, synced: &synced, sync_directory };
                let result = replace(&self.swept, &path, bytes, &steps);
                complete.complete(result);
                self.release(&path);
            }
            Request::ReadDir { path, complete } => {
                let result = read_dir(&path);
                if let Ok(entries) = &result {
                    span.record("entries", entries.len() as u64);
                }
                complete.complete(result);
            }
            Request::CreateDirAll { path, complete } => {
                complete.complete(create_dir_all(&path));
            }
        }
    }

    /// Moves the next replace of `path` from its waitlist to the back of the queue, after
    /// one of `path` has finished or been skipped; drops, unrun, every one whose future was
    /// dropped while it waited; removes the key when none is left.
    fn release(&self, path: &Path) {
        let mut dropped = Vec::new();
        {
            let mut guard = lock(&self.queue);
            let queue = &mut *guard;
            // Absent after a stop, which failed the waitlist with the rest of the queue.
            let Some(waiting) = queue.replacing.get_mut(path) else { return };
            loop {
                match waiting.pop_front() {
                    None => {
                        queue.replacing.remove(path);
                        break;
                    }
                    Some(next) if next.is_cancelled() => dropped.push(next),
                    Some(next) => {
                        queue.requests.push_back(next);
                        self.available.notify_one();
                        break;
                    }
                }
            }
        }
        drop(dropped);
    }

    #[cfg(test)]
    fn observe(&self, point: Point<'_>) {
        if let Some(hook) = &self.hook {
            hook(point);
        }
    }
}

impl Backend for BlockingPool {
    fn submit(&self, request: Request) {
        #[cfg(test)]
        self.observe(Point::Submitted(&request));
        let mut guard = lock(&self.queue);
        let queue = &mut *guard;
        if queue.stopping {
            drop(guard);
            request.cancel();
            return;
        }
        if let Some(path) = request.replace_path() {
            if let Some(waiting) = queue.replacing.get_mut(path) {
                waiting.push_back(request);
                return;
            }
            queue.replacing.insert(path.to_path_buf(), VecDeque::new());
        }
        queue.requests.push_back(request);
        drop(guard);
        self.available.notify_one();
    }

    fn stop(&self) {
        let drained: Vec<Request> = {
            let mut guard = lock(&self.queue);
            let queue = &mut *guard;
            queue.stopping = true;
            let mut drained: Vec<Request> = queue.requests.drain(..).collect();
            for (_, waiting) in queue.replacing.drain() {
                drained.extend(waiting);
            }
            drained
        };
        self.available.notify_all();
        // Outside the lock: a completion's wake may run a task inline that submits again.
        for request in drained {
            request.cancel();
        }
    }

    fn join(&self) {
        self.stop();
        // Held across the joins, so a second `join` that runs at once returns only once
        // every thread has exited too. No pool thread takes this lock.
        let mut threads = lock(&self.threads);
        for thread in threads.drain(..) {
            thread.join();
        }
    }
}

/// A lock whose poisoning is ignored: panics abort outside tests, and every update under
/// the pool's locks leaves its state consistent.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The kinds each request returns (`README.md`, "Errors"), `Cancelled` aside, which only a
/// stopped pool gives.
const OPEN: &[ErrorKind] = &[ErrorKind::NotFound, ErrorKind::ResourceExhausted, ErrorKind::Io];
const READ: &[ErrorKind] = &[ErrorKind::InvalidData, ErrorKind::ResourceExhausted, ErrorKind::Io];
const READ_FILE: &[ErrorKind] =
    &[ErrorKind::NotFound, ErrorKind::InvalidData, ErrorKind::ResourceExhausted, ErrorKind::Io];
const REPLACE: &[ErrorKind] =
    &[ErrorKind::NotFound, ErrorKind::StorageFull, ErrorKind::ResourceExhausted, ErrorKind::Io];
const READ_DIR: &[ErrorKind] = &[ErrorKind::NotFound, ErrorKind::ResourceExhausted, ErrorKind::Io];
const CREATE_DIR_ALL: &[ErrorKind] =
    &[ErrorKind::StorageFull, ErrorKind::ResourceExhausted, ErrorKind::Io];

/// A system call's error as a request's: `From<std::io::Error>`, keeping the source, with
/// every kind outside `kinds` made `Io`. Among them are the two kinds no request returns:
/// `Unsupported` arises from `ENOTSUP` (an `F_FULLFSYNC` on a file system without it) and
/// `Network` from a timeout or a reset on a network file system; neither is a policy `app`
/// has for a file.
pub(crate) fn os_error(error: std::io::Error, kinds: &[ErrorKind]) -> Error {
    let error = Error::from(error);
    if kinds.contains(&error.kind()) {
        error
    } else {
        Error::new(ErrorKind::Io, error.message().to_owned()).with_source(error)
    }
}

/// `InvalidData` for a range or a file that is not what the request needs.
fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// A request's length as a buffer's: `ResourceExhausted` where it does not fit `usize`.
fn buffer_len(len: u64) -> Result<usize> {
    usize::try_from(len).map_err(|_| {
        Error::new(ErrorKind::ResourceExhausted, format!("{len} bytes do not fit in memory"))
    })
}

/// An empty buffer with room for exactly `len` bytes, or `ResourceExhausted` if the
/// allocator refuses it: never an abort.
fn allocate(len: usize) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len).map_err(|source| {
        Error::new(ErrorKind::ResourceExhausted, format!("no memory for {len} bytes"))
            .with_source(source)
    })?;
    Ok(buffer)
}

/// Opens `path` for reading and records its size. `NotFound` if there is no such file;
/// `Io` for a directory (which Unix opens for reading), a permission failure or anything
/// else `std::fs::File::open` reports.
fn open(backend: &Arc<dyn Backend>, path: PathBuf, id: FileId) -> Result<Arc<OpenFile>> {
    let file = std::fs::File::open(&path).map_err(|error| os_error(error, OPEN))?;
    let metadata = file.metadata().map_err(|error| os_error(error, OPEN))?;
    if metadata.is_dir() {
        return Err(Error::new(ErrorKind::Io, format!("{} is a directory", path.display())));
    }
    Ok(Arc::new(OpenFile { file, path, size: metadata.len(), id, backend: Arc::clone(backend) }))
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
    let size = file.file.metadata().map_err(|error| os_error(error, READ))?.len();
    let end = offset.checked_add(len).ok_or_else(|| invalid("the range overflows"))?;
    if end > size {
        return Err(invalid(format!(
            "the range {offset}..{end} runs past the end of {}, {size} bytes",
            file.path.display()
        )));
    }
    let len = buffer_len(len)?;
    let mut buffer = allocate(len)?;
    // Within the reserved capacity: zero-filled, since a positional read takes an
    // initialised slice.
    buffer.resize(len, 0);
    read_exact_at(&file.file, &mut buffer, offset).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            invalid(format!("{} shrank under the read", file.path.display())).with_source(error)
        } else {
            os_error(error, READ)
        }
    })?;
    Ok(buffer)
}

#[cfg(unix)]
fn read_exact_at(file: &std::fs::File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(
    file: &std::fs::File,
    mut buffer: &mut [u8],
    mut offset: u64,
) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buffer.is_empty() {
        match file.seek_read(buffer, offset) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => {
                buffer = &mut buffer[read..];
                offset += read as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The pool's part of an encoded read: the stored frame read and checked, and the decoded
/// buffer reserved, for the future to decode in its own poll ([`Encoded::Stored`]):
///
/// 1. [`read_at`] of the stored range (fstat first, a fallible allocation of `len`).
/// 2. Before anything else is allocated, the frame checks of [`decode::check_frame`], each
///    failure `InvalidData`: one whole, non-skippable frame, its content size
///    `decoded_len`.
/// 3. `Vec::try_reserve_exact(decoded_len)` for the output: a failure is
///    `ResourceExhausted`.
fn read_encoded_at(file: &OpenFile, offset: u64, len: u64, encoding: Encoding) -> Result<Encoded> {
    let Encoding::Zstd { decoded_len } = encoding;
    let stored = read_at(file, offset, len)?;
    decode::check_frame(&stored, decoded_len)?;
    let decoded_len = buffer_len(decoded_len)?;
    let decoded = allocate(decoded_len)?;
    Ok(Encoded::Stored { stored, decoded, decoded_len })
}

/// The whole file at `path`; see [`read_file_with`].
fn read_file(path: &Path) -> Result<Vec<u8>> {
    read_file_with(path, || {})
}

/// The whole file at `path`: open it, take its size from `metadata()` (fstat),
/// `ResourceExhausted` if it exceeds `MAX_READ_BYTES` or `try_reserve_exact` of it fails,
/// before anything is read; then, after `between` (nothing but in a test), read exactly
/// that many bytes (`read_exact`: a file that shrank is `UnexpectedEof`, so `InvalidData`)
/// and try to read one more. A byte read there means the file grew while it was read, and
/// the request is `InvalidData` with the buffer dropped, never a buffer past the cap and
/// never a silent truncation. A caller that reads a file being written retries.
fn read_file_with(path: &Path, between: impl FnOnce()) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(path).map_err(|error| os_error(error, READ_FILE))?;
    let metadata = file.metadata().map_err(|error| os_error(error, READ_FILE))?;
    if metadata.is_dir() {
        return Err(Error::new(ErrorKind::Io, format!("{} is a directory", path.display())));
    }
    let size = metadata.len();
    if size > MAX_READ_BYTES {
        return Err(Error::new(
            ErrorKind::ResourceExhausted,
            format!(
                "{} is {size} bytes, above the {MAX_READ_BYTES} a request may read",
                path.display()
            ),
        ));
    }
    let size = buffer_len(size)?;
    let mut buffer = allocate(size)?;
    buffer.resize(size, 0);
    between();
    let changed = || invalid(format!("{} changed size while it was read", path.display()));
    file.read_exact(&mut buffer).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            changed().with_source(error)
        } else {
            os_error(error, READ_FILE)
        }
    })?;
    let mut probe = [0u8; 1];
    loop {
        match file.read(&mut probe) {
            Ok(0) => return Ok(buffer),
            Ok(_) => return Err(changed()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(os_error(error, READ_FILE)),
        }
    }
}

/// Architecture section 15.3's atomic replace, as one request, with the bytes handed back
/// in the error on every failure:
///
/// 0. The directory is `path.parent()`, and `"."` when that is empty (`slot0.carton`). A
///    `path` with no file name is `Io` before anything is touched.
/// 1. [`sweep`] the directory, once per directory per pool.
/// 2. Create `<name>.<token><counter>.tmp` in the directory with `create_new`, so a
///    leftover is never reused; `<token>` is [`token`] and `<counter>` the process-wide
///    [`COUNTER`], each as eight lower-case hex digits. On `AlreadyExists` take the next
///    counter value and try again, [`CREATE_ATTEMPTS`] times in all, then `Io`. `NotFound`
///    if the directory does not exist. `steps.created` is told the temporary's path.
/// 3. `write_all(bytes)`, then [`sync_file`] (`sync_all`: `fsync`; `F_FULLFSYNC` on Apple
///    platforms, which `std` issues for `sync_all`; `FlushFileBuffers` on Windows).
///    `StorageFull` from either step.
/// 4. `rename` the temporary over `path`. Same directory, so never across volumes.
/// 5. `steps.sync_directory`, which is [`sync_directory`] but in a test: on Unix, open the
///    directory and `sync_all` it, so the rename is durable too.
///
/// A failure in steps 2 to 4 removes the temporary file (best effort) and leaves `path` as
/// it was, with the kind [`os_error`] gives. A failure in step 5 is `Io` whatever the
/// system call said (a directory removed after the rename would otherwise be `NotFound`,
/// which `app` reads as "no save directory"): the new content is in place and its
/// durability unknown. A started replace runs every step however its future was dropped.
fn replace(
    swept: &Mutex<HashSet<PathBuf>>,
    path: &Path,
    bytes: Vec<u8>,
    steps: &Steps<'_>,
) -> Result<(), ReplaceFailed> {
    match write_and_rename(swept, path, &bytes, steps) {
        Ok(()) => Ok(()),
        Err(error) => Err(ReplaceFailed { error, bytes }),
    }
}

/// What a replace tells of its steps, and how it syncs the directory: the pool passes
/// observers that do nothing outside the crate's tests, and [`sync_directory`].
struct Steps<'a> {
    /// Told the temporary's path once it is created, before anything is written to it.
    created: &'a Told<'a>,
    /// Told by [`sync_file`] and [`sync_directory`] what they synced.
    synced: &'a Told<'a>,
    /// The directory sync after the rename, given `synced`.
    sync_directory: fn(&Path, &Told<'_>) -> std::io::Result<()>,
}

/// An observer of one kind of step of a replace, told the path the step was about.
type Told<'a> = dyn Fn(&Path) + 'a;

fn write_and_rename(
    swept: &Mutex<HashSet<PathBuf>>,
    path: &Path,
    bytes: &[u8],
    steps: &Steps<'_>,
) -> Result<()> {
    let Some(name) = path.file_name() else {
        return Err(Error::new(ErrorKind::Io, format!("{} names no file", path.display())));
    };
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    sweep(swept, directory);
    let (temporary, mut file) = create_temporary(directory, name)?;
    (steps.created)(&temporary);
    let written = file.write_all(bytes).and_then(|()| sync_file(&file, &temporary, steps.synced));
    // Closed before the rename: Windows renames an open file only with delete sharing.
    drop(file);
    if let Err(error) = written.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(os_error(error, REPLACE));
    }
    (steps.sync_directory)(directory, steps.synced).map_err(|error| {
        Error::new(
            ErrorKind::Io,
            format!(
                "{} was replaced but syncing its directory failed: its durability is unknown",
                path.display()
            ),
        )
        .with_source(error)
    })
}

/// Creates a new temporary for `name` in `directory`, skipping names already taken.
fn create_temporary(directory: &Path, name: &OsStr) -> Result<(PathBuf, std::fs::File)> {
    let mut taken = None;
    for _ in 0..CREATE_ATTEMPTS {
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut file_name = name.to_os_string();
        file_name.push(format!(".{:08x}{counter:08x}.tmp", token()));
        let temporary = directory.join(file_name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => taken = Some(error),
            Err(error) => return Err(os_error(error, REPLACE)),
        }
    }
    let error = Error::new(
        ErrorKind::Io,
        format!("every temporary name tried in {} was taken", directory.display()),
    );
    Err(match taken {
        Some(source) => error.with_source(source),
        None => error,
    })
}

/// `file.sync_all()`, then `synced` told `path`.
fn sync_file(file: &std::fs::File, path: &Path, synced: &Told<'_>) -> std::io::Result<()> {
    file.sync_all()?;
    synced(path);
    Ok(())
}

/// The directory sync after a rename: on Unix the directory opened and `sync_all`ed, so the
/// rename itself is durable; then `synced` told `directory`. Windows has no directory sync;
/// `io` does not attempt one.
fn sync_directory(directory: &Path, synced: &Told<'_>) -> std::io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    synced(directory);
    Ok(())
}

/// The first replace into `directory` through this pool, keyed as spelled, inserts its key
/// and lists it, removing every regular file named `<any>.<token><counter>.tmp` (sixteen
/// lower-case hex digits in all, `<any>` not empty) whose `<token>` is not this process's
/// [`token`], whatever its `<any>`: the leftovers of an earlier process, a crashed one or
/// an earlier run of a container's pid 1 among them. Best effort: a listing or removal that
/// fails is ignored, and the create that follows reports the directory's real state. It
/// never removes a temporary of this process, so no replace of this process, under any
/// spelling and from any `Io`, removes another's. A second process writing into the same
/// directory is unsupported: its live temporaries carry a foreign token.
fn sweep(swept: &Mutex<HashSet<PathBuf>>, directory: &Path) {
    if !lock(swept).insert(directory.to_path_buf()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        if is_foreign_temporary(&entry.file_name(), token())
            && entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether `name` is `<any>.<sixteen lower-case hex digits>.tmp`, `<any>` not empty, whose
/// first eight digits are not `token`'s.
pub(crate) fn is_foreign_temporary(name: &OsStr, token: u32) -> bool {
    const DIGITS: usize = 16;
    let Some(stem) = name.as_encoded_bytes().strip_suffix(b".tmp") else { return false };
    let Some(split) = stem.len().checked_sub(DIGITS) else { return false };
    let (head, unique) = stem.split_at(split);
    let Some(any) = head.strip_suffix(b".") else { return false };
    if any.is_empty() || !unique.iter().all(|digit| matches!(digit, b'0'..=b'9' | b'a'..=b'f')) {
        return false;
    }
    unique[..8] != *format!("{token:08x}").as_bytes()
}

/// Every entry of the directory at `path`, by its own file type (a symbolic link is
/// `Other`), sorted by name. `NotFound` if there is no such directory; `Io` if `path` is
/// not a directory or cannot be read. An entry removed while the listing runs is left
/// out. A `.tmp` left by a crashed [`replace`] is listed like any other file until a later
/// process's first replace into its directory removes it; callers that list slots ignore
/// the suffix.
fn read_dir(path: &Path) -> Result<Vec<DirEntry>> {
    // Past the open, a `NotFound` is an entry removed under the listing, not the directory.
    const ENTRY: &[ErrorKind] = &[ErrorKind::ResourceExhausted, ErrorKind::Io];
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|error| os_error(error, READ_DIR))? {
        let entry = entry.map_err(|error| os_error(error, ENTRY))?;
        let kind = match entry.file_type() {
            Ok(kind) if kind.is_file() => EntryKind::File,
            Ok(kind) if kind.is_dir() => EntryKind::Directory,
            Ok(_) => EntryKind::Other,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(os_error(error, ENTRY)),
        };
        entries.push(DirEntry { name: entry.file_name(), kind });
    }
    entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// `std::fs::create_dir_all`: every missing directory of `path`, and success if all of
/// them exist already. `Io` on a permission failure or a file in the way, and for a
/// `NotFound` (a directory on the way removed while it ran); `StorageFull` for a full disk.
fn create_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|error| os_error(error, CREATE_DIR_ALL))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tests::Scratch;

    #[test]
    fn unsupported_and_network_become_io_and_keep_their_source() {
        use std::io::ErrorKind as Os;
        let every = &[
            ErrorKind::NotFound,
            ErrorKind::InvalidData,
            ErrorKind::StorageFull,
            ErrorKind::ResourceExhausted,
            ErrorKind::Io,
        ];
        for (os, kind) in [
            (Os::Unsupported, ErrorKind::Io),
            (Os::TimedOut, ErrorKind::Io),
            (Os::ConnectionReset, ErrorKind::Io),
            (Os::ConnectionRefused, ErrorKind::Io),
            (Os::NotConnected, ErrorKind::Io),
            (Os::NotFound, ErrorKind::NotFound),
            (Os::StorageFull, ErrorKind::StorageFull),
            (Os::QuotaExceeded, ErrorKind::StorageFull),
            (Os::OutOfMemory, ErrorKind::ResourceExhausted),
            (Os::UnexpectedEof, ErrorKind::InvalidData),
            (Os::PermissionDenied, ErrorKind::Io),
        ] {
            let error = os_error(std::io::Error::from(os), every);
            assert_eq!(error.kind(), kind, "{os:?}");
            assert!(std::error::Error::source(&error).is_some(), "{os:?} lost its source");
        }
        // Every kind outside a request's list is `Io`: `create_dir_all` returns no `NotFound`.
        let error = os_error(std::io::Error::from(Os::NotFound), CREATE_DIR_ALL);
        assert_eq!(error.kind(), ErrorKind::Io);
    }

    #[test]
    fn the_temporary_pattern_is_any_name_sixteen_hex_digits_and_tmp() {
        let token = 0x0123_abcd;
        let foreign = |name: &str| is_foreign_temporary(OsStr::new(name), token);
        assert!(foreign("slot0.carton.89abcdef00000001.tmp"));
        assert!(foreign("x.ffffffffffffffff.tmp"));
        assert!(!foreign("slot0.carton.0123abcd00000001.tmp"), "this process's token");
        assert!(!foreign("slot0.carton.bak.tmp"));
        assert!(!foreign("slot0.carton.89abcdef0000001.tmp"), "fifteen digits");
        assert!(!foreign("slot0.carton.89abcdef000000001.tmp"), "seventeen digits");
        assert!(!foreign(".89abcdef00000001.tmp"), "an empty name");
        assert!(!foreign("89abcdef00000001.tmp"), "no name at all");
        assert!(!foreign("slot0.89ABCDEF00000001.tmp"), "upper-case digits");
        assert!(!foreign("slot0.89abcdef00000001.tmpx"));
    }

    #[test]
    fn a_file_appended_to_while_it_is_read_is_invalid_data() {
        let scratch = Scratch::new();
        let path = scratch.path("growing");
        std::fs::write(&path, b"0123456789").unwrap();
        let append = || {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(b"more").unwrap();
        };
        let error = read_file_with(&path, append).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        // A file truncated in the same place is `InvalidData` too.
        let truncate = || std::fs::write(&path, b"01").unwrap();
        let error = read_file_with(&path, truncate).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        // Untouched, it reads whole.
        assert_eq!(read_file(&path).unwrap(), b"01");
    }

    #[test]
    fn a_failure_after_the_rename_is_io_with_the_new_content_in_place() {
        let scratch = Scratch::new();
        let path = scratch.path("slot");
        std::fs::write(&path, b"old").unwrap();
        let swept = Mutex::new(HashSet::new());
        let fails =
            |_: &Path, _: &Told<'_>| Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        let steps = Steps { created: &|_| {}, synced: &|_| {}, sync_directory: fails };
        let bytes = b"new".to_vec();
        let pointer = bytes.as_ptr();
        let failed = replace(&swept, &path, bytes, &steps).unwrap_err();
        assert_eq!(failed.error.kind(), ErrorKind::Io, "{}", failed.error);
        assert_eq!(failed.bytes.as_ptr(), pointer);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(scratch.temporaries(), 0);
    }
}
