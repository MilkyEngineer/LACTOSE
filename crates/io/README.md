# lactose-io

File IO (architecture section 4, `io`): whole files and positional ranges of a file read
into caller-owned buffers, a range decoded in the same request, the atomic replace of
section 15.3, and directory listing, each a `Send + 'static` future completed by a pool of
platform threads of class `ThreadClass::Io`. SILO reads packs through it, `app` writes
and reads saves through it, the cooker reads sources and writes outputs through it. The
simulation never calls it.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]`; `SAFETY.md` says where the completion backends' `unsafe` will live |

**Status: facade.** Every public signature is final and every deciding body is `todo!()`;
`IoConfig::for_topology`, `IoConfig::for_platform` and the span constructors are real.
Every sentence under "Behaviour the signatures cannot express" is one a dependent crate
may rely on and the implementation must meet. Architecture sections 3, 4, 5.1, 9.7, 14,
15.3, 19, 20, 21 and 22 and `docs/verification.md` section 3 are the design; where this
file and the architecture disagree, the architecture wins and this file is wrong.
Decision D130 records the choices those sections left open. `crates/core/README.md`
(errors, `AllocTag`, `ThreadClass`) and `crates/task/README.md` (the executor, `Task`,
`Platform`, `block_on`) are the contracts this crate builds on and never restates.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `asset`, `persist`'s callers in `app`, `cook`
and `app` are written against this surface. `tests/sketch.rs` is a compile-only sketch of
what each of them writes: a change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Io` | The pool handle: `Clone`, one per process, started by `app` with `Io::start(platform, config)` and cloned into whatever submits requests; `open`, `read_file`, `replace`, `read_dir`, `create_dir_all` are the requests that take a path; `pool_threads`; `shutdown(self)` |
| `IoConfig` | The one setting, `pool_threads: NonZero<u32>`; `for_platform` and `for_topology` are section 9.7's rule, half the cores rounded up, at least one, the same on client and server |
| `File`, `FileId` | An open file: `Clone` over one descriptor, `path`, `size` at the open, `id` for the spans; `read_at(offset, len)` and `read_encoded_at(offset, len, encoding)` are the positional reads, any number in flight at once |
| `Encoding` | How a stored range is decoded in the same request: `Zstd { decoded_len }`, the exact decoded length from the index that named the range; `#[non_exhaustive]` for a platform's own format |
| `Open`, `Read`, `ReadFile`, `Replace`, `ReadDir`, `CreateDirAll` | The futures, one per request: `Send + Sync + Unpin + 'static`, submitted at the first poll, cancelled by drop; each resolves to all of what was asked or an error |
| `DirEntry`, `EntryKind` | One entry of a listing: its own name and its own kind (`File`, `Directory`, `Other`), sorted by name |
| `MAX_READ_BYTES` | 1 GiB, the most one read may ask for stored or decoded; above it `ResourceExhausted` before any allocation |

Everything else is hidden and changeable: the `Backend` trait (architecture section 20:
"its backends sit behind a trait inside the crate"), the blocking pool behind it, the
request enum, the completion a future and its backend share, the span constructors.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| SILO (`asset`) | A pack opened once for the session and read in ranges named by its index; each range into a buffer the store owns, as a `'static` future it spawns per load; the range decoded in the same request so a console's hardware decoder serves it unchanged (`docs/verification.md` section 3); many reads in flight on one handle; a load abandoned by dropping its task; a corrupt index entry failing as a value, never as an allocation | `Io::open`, `File::read_at`, `File::read_encoded_at`, `Read` (`Send + 'static`, cancelled by drop), `MAX_READ_BYTES`; the loader runs over the returned bytes on the worker that awaited them |
| `app`, saves (SKIM, CARTON) | The bytes `Container::assemble` produced written as one request that is atomic and durable or has not happened, with failure modes `app` chooses policy from by kind: disk full, missing directory, permissions; the preamble then the sections as positional reads of one open slot; the slot listing; the save directory created at startup | `Io::replace` and its error table, `Io::open` + `File::read_at`, `Io::read_dir`, `Io::create_dir_all` |
| The cooker (RICOTTA) | Sources listed and read, outputs written whole, from a tool with no scheduler of its own | The same futures through `Executor::block_on`; there is no second, blocking API |
| The server (section 9.7) | A pool sized to half the cores, separate from the background workers, so a region publish never waits behind another; threads from the platform, not `std::thread` | `IoConfig::for_platform`, `Io::start` through `Platform::spawn_thread` with `ThreadClass::Io`, `pool_threads` requests in flight at once |
| Profiling (section 3) | One span per request, target `io`, with the fields a profiler joins on | The span table below |
| The executor (WHISK) | Nothing of it is used but the platform for threads; a request's future is a plain `std` future woken through its `Waker`, so any poll loop drives it | `Io::start(&dyn Platform, ..)`; the futures are `Unpin` |

## Behaviour the signatures cannot express

### Requests and futures

- Every request is a future, and nothing is submitted until it is first polled: no queue
  entry, no system call. A future created and dropped unpolled has cost its own
  allocation and its span, and nothing else.
- A future resolves exactly once, to all of what was asked or to an error. A read never
  resolves to fewer bytes than its `len`; a replace never resolves `Ok` with the file half
  written; a listing never resolves to part of a directory.
- Every future is `Send + Sync + Unpin + 'static` and owns everything it needs: the path,
  the bytes to write, a clone of the file handle. It is awaited in place, spawned with
  `Executor::spawn` as it is, or driven by `Executor::block_on`. Dropping a `Task` that
  wraps one drops the future.
- Dropping a future cancels its request. One not yet started by the pool never starts and
  has no side effect: no file is opened, no byte is read or written. One already started
  runs to its end on the pool thread and its result is discarded there: a read's buffer is
  freed on the pool thread, an open's descriptor is closed there, a replace completes its
  rename or its cleanup. The drop itself never blocks and never waits for the pool.
- One future is one request, and a future awaited to its end before the next is created
  or polled gives one request in flight. Several requests in flight at once are several
  futures polled at once: one `Task` per request through `Executor::spawn`, or any poll
  loop that polls them together. A loop that awaits them one after another runs them one
  after another.
- Requests start in the order their futures were first polled (one FIFO queue) and
  complete in no particular order. The one ordering guarantee between requests is through
  the caller: a request's effects are visible to every request whose future is first
  polled after the first request's future resolved. Two requests in flight at once on the
  same path are unordered.
- There is no priority. `LoadPriority` in `asset-api` orders what SILO submits, not what
  this crate runs; the IO budget of milestone 3 decides whether that changes.
- Each request's own bookkeeping (the future, the request record, the completion) is
  allocated on the caller's thread under the caller's `AllocTag`. Every buffer a request
  returns is allocated on the pool thread under `AllocTag::Io`; freeing it anywhere is
  charged back to `Io`.

### Reads

- `File::read_at(offset, len)` resolves to exactly `len` bytes read from `offset`, in a
  `Vec<u8>` whose length and capacity are `len`, owned by the caller. The file ending
  before `offset + len` is `InvalidData`, with no partial buffer; `offset + len`
  overflowing `u64` is `InvalidData`; `len` above `MAX_READ_BYTES` is `ResourceExhausted`,
  decided before anything is allocated or submitted. A `len` of zero resolves to an empty
  buffer without a system call, whatever the offset.
- `File::read_encoded_at(offset, len, encoding)` reads the `len` stored bytes at `offset`
  and decodes them on the pool thread, as one request, into a `Vec<u8>` whose length and
  capacity are `encoding`'s `decoded_len`. The stored bytes are never handed out. The
  stored bytes must be exactly one frame of `encoding`, starting at `offset` and ending at
  `offset + len`; a truncated frame, trailing bytes after it, a frame that decodes to any
  length but `decoded_len`, or a frame whose own content-size header disagrees with
  `decoded_len`, is `InvalidData`. `decoded_len` is the caller's claim from the index that
  named the range, and is what is trusted for the allocation; the frame is never allowed
  to grow the buffer past it. `len` or `decoded_len` above `MAX_READ_BYTES` is
  `ResourceExhausted` before anything is allocated. Two allocations under `AllocTag::Io`:
  the stored buffer, freed on the pool thread, and the decoded one, returned.
- `Io::read_file(path)` resolves to the file's whole content as it is when the read runs.
  A file whose size exceeds `MAX_READ_BYTES` is `ResourceExhausted` before anything is
  allocated. It is the one-shot form; a file read in ranges is opened once.
- A `File` is read-only, positional and cursorless: reads on it share nothing but the
  descriptor, and any number may be in flight at once, on any threads. `File::size()` is
  the size at the open; a read checks its range against the file, not against `size()`.
- A `File` stays open while any clone or any read in flight on it exists, and closes when
  the last is dropped, on that thread, without a request. A file replaced while a handle
  on it is open keeps serving the content the handle was opened on: the previous content
  on Unix, where the rename unlinks the name and not the inode, and on Windows, where
  `std` opens with delete sharing so the rename succeeds and the handle keeps the old
  file. Packs are immutable and saves are only ever replaced, so no caller sees a file
  change under a handle.
- A read on a `File` whose pool has stopped is `Cancelled`.

### The atomic replace

- `Io::replace(path, bytes)` is architecture section 15.3's operation as one request. On
  the pool thread: a temporary file `<name>.<unique>.tmp` is created in the same directory
  as `path` (so the rename is never across volumes) with `create_new`, so a leftover of
  another attempt is never reused; `bytes` is written to it in full; the temporary is
  synced (`fsync`, which `std` issues as `F_FULLFSYNC` on Apple platforms, and
  `FlushFileBuffers` on Windows); it is renamed over `path`; and on Unix the directory is
  synced so the rename itself is durable.
- When the future resolves `Ok`, a reader of `path` on this machine sees all of `bytes`,
  and a crash or power loss after that point keeps them, to the extent the platform's
  rename is durable (`docs/verification.md` has the row). At no point does a reader of
  `path` see anything but the previous content or the new: there is no moment at which
  `path` is missing or partial.
- On any failure, `path` keeps its previous content and the temporary file is removed
  (best effort: a crash between the failure and the removal can leave it). The kinds:
  `StorageFull` when the disk or quota fills during the write or the sync; `NotFound` when
  `path`'s directory does not exist, which callers prevent with `Io::create_dir_all` at
  startup rather than retry; `Io` for a permission failure, for a `path` that is a
  directory, and on Windows for a `path` another process holds open without delete
  sharing. A cross-volume rename cannot happen.
- `bytes` is consumed by the request. A caller that may retry after a failure keeps its
  own copy or produces the bytes again.
- A `Replace` dropped before it started never touches the disk. One dropped after it
  started runs every step, so the path is replaced or left as it was and the temporary is
  cleaned up; the drop only discards the result.
- A `.tmp` left by a crash is an ordinary file to `Io::read_dir`. Callers that list a
  directory of slots ignore names ending in `.tmp`.

### Directories

- `Io::read_dir(path)` resolves to every entry of the directory, excluding `.` and `..`,
  sorted by `name` as `OsString` orders (byte order of the name), each with the kind of
  the entry itself: a symbolic link is `EntryKind::Other` whatever it points to, and so is
  anything that is neither a regular file nor a directory. `NotFound` if there is no such
  directory; `Io` if `path` is not a directory or cannot be read. A directory changed
  while it is being listed gives some listing of it; the sort is of what was read.
- `Io::create_dir_all(path)` creates every missing directory on the way to `path` and
  resolves `Ok` when `path` is a directory, including when it already was. `Io` on a
  permission failure or when a file is in the way.

### The pool

- `Io::start(platform, config)` creates `config.pool_threads` threads through
  `platform.spawn_thread`, each `ThreadSpec { name: "io-<index>", class: ThreadClass::Io,
  index_in_class: index }` with `index` from zero, never through `std::thread`
  (architecture section 5.1). Each thread holds an `AllocTag::Io` scope for its whole
  life. `ThreadClass::Io` is utility priority like `Background`, its own class so a
  platform that pins by class and index never puts an IO thread on a background worker's
  core. The platform is used for the spawns and not kept.
- `Io::start` fails with what the platform reports if a thread cannot be created, after
  stopping the threads it did create. Nothing else fails at start.
- Each thread takes one request at a time from the one FIFO queue and runs it from its
  first system call to its result, decoding included. `pool_threads` is therefore the
  number of requests in flight at once; a request waits for a thread, never for another
  request's completion.
- `IoConfig::for_topology` is `(performance + efficiency).div_ceil(2).max(1)`: four
  threads on a 4+4 part, eight on a server the topology reports as fifteen homogeneous
  cores, one on a four-vCPU runner. Client and server use the same rule until a
  measurement says otherwise (`docs/verification.md`); `app` may set `pool_threads`
  directly.
- Decoding runs on the pool thread, not on a background worker. The thread is asleep in
  a system call for most of its life, the decode of one range is short next to the read
  that fetched it, and a platform with a hardware decoder completes the request as a whole
  (D130). `docs/verification.md` measures whether a client's pool contends with its
  frame.
- `Io` is `Clone`, and every clone is the one pool. `Io::shutdown(self)` stops it:
  requests already started finish; requests queued, and requests submitted after it from
  any clone, resolve `Cancelled`; then it blocks until every thread has exited. It is for
  the main thread and tools, before `Executor::shutdown`. Clones left after it are inert
  handles whose requests resolve `Cancelled`.
- Dropping the last `Io` clone without `shutdown` stops the pool the same way but does not
  wait: the threads exit after their current request and are not joined. Open `File`s
  outlive it as handles whose reads resolve `Cancelled`.
- No request ever runs on the caller's thread, blocks the caller's thread, or waits for
  the executor. The pool never calls into the executor; a request's future is woken
  through the `Waker` of its last poll, so `Executor::block_on` and a hand-written poll
  loop drive it as well as a spawned task does.

### Errors

Every failure is a `lactose_core::Error` whose kind is the one policy dispatches on;
`std::io::Error`s map through `From<std::io::Error>` and keep their source.

| Kind | When |
|---|---|
| `NotFound` | `open`, `read_file`: no such file. `read_dir`: no such directory. `replace`: the directory of `path` does not exist |
| `InvalidData` | A read whose range runs past the end of the file or overflows; a decode of anything but one whole frame of exactly `decoded_len` bytes |
| `ResourceExhausted` | A `len`, `decoded_len` or whole-file size above `MAX_READ_BYTES`, before any allocation |
| `StorageFull` | `replace`: the disk or quota filled during the write or the sync |
| `Io` | Anything else the operating system reports: permissions, a directory where a file was named or a file where a directory was, a Windows sharing violation |
| `Cancelled` | A request submitted to a pool that has stopped, or queued when it stopped. Never from dropping the future: a dropped future has no one to tell |

`Unsupported`, `VersionMismatch`, `Network`, `DeviceLost` and `Platform` are never returned
by a request. `Io::start` returns the platform's own error, of whatever kind the platform
gives a thread it cannot create (`Io` from `StdPlatform`).

### Tracing

One span per request, target `io`, level `INFO`, created with the future, entered on the
pool thread for the work, and closed when the request's state is released (after the
result is taken or the future is dropped), so a profiler sees the time waiting and the
time working. A future dropped unpolled closes its span without ever entering it. `error`
is recorded on completion with the kind, for a failure only.

| Span | Fields at creation | Recorded at completion |
|---|---|---|
| `io.open` | `path` | `file` (the `FileId`), `size`, `error` |
| `io.read` | `file`, `offset`, `len`, `encoding` (`"raw"` or `"zstd"`), `decoded_len` (equal to `len` for a raw read) | `error` |
| `io.read_file` | `path` | `len`, `error` |
| `io.replace` | `path`, `len` | `error` |
| `io.read_dir` | `path` | `entries`, `error` |
| `io.create_dir_all` | `path` | `error` |

Nothing else is logged at `INFO` or above. A request's start and finish are the span's
own events.

## Not in this crate yet

- **io_uring and IOCP backends** (architecture section 4). The `Backend` trait and
  `src/backend/` are where they go; `SAFETY.md` is where their `unsafe` is argued;
  `docs/verification.md` has the CI spike row. Until one lands every platform runs the
  blocking pool.
- **Priority and an IO budget.** The queue is FIFO. Milestone 3's streaming decides, with
  `asset-api`'s budget type, whether a request carries a priority and whether the pool
  reorders; SILO's `LoadPriority` orders its own submissions until then.
- **Writes other than the atomic replace**: appending to a file, writing a range, a pack
  larger than memory streamed out by the cooker. The cooker writes each output whole; a
  pack that no longer fits is milestone 3's problem and will want an appending writer with
  its own sync and rename story.
- **Reads into a caller-provided buffer.** Every read allocates its result. SILO's loaders
  take `&[u8]` and build the asset from it, so the read buffer is a transient and the
  allocation is the simplest ownership; a zero-copy read into an asset's final storage
  waits for a consumer that needs one.
- **Deleting and renaming files, and removing directories.** No consumer of milestone 1
  deletes: saves are replaced, packs are immutable, the cooker writes over its outputs.
  A save-slot delete in the UI is the first consumer and brings `remove_file` with it.
- **Metadata without opening** (size, modified time) and **watching a directory** for the
  cooker's hot reload (`asset`'s milestone 2 work). Both are requests of the same shape
  and arrive with their consumer.
- **A platform's own compression format** as a second `Encoding` variant, when a console
  backend brings a decoder for it.

## Proof for milestone 1

Tests the implementation writes, all under `cargo nextest`, none under SOUR: this crate
is outside the determinism boundary and nothing in it hashes.

- **Config.** `for_topology` on the listed topologies (exists).
- **Cancellation.** A future created and dropped unpolled submits nothing: the queue is
  untouched and its span is never entered. A future dropped while queued never runs: with one
  pool thread held by a slow request, a queued replace is dropped and the path is never
  created. A future dropped after its request started completes on the pool: no panic, the
  buffer is freed, `AllocTag::Io` live bytes return to zero. The drop returns at once
  while the pool thread is still inside the request.
- **Ordering.** With one pool thread, requests start in first-poll order. A replace
  awaited to `Ok` followed by a `read_file` of the same path sees the new content.
- **Errors.** Each row of the table is produced: a missing file, a missing directory for
  a replace, a range past the end (and no partial buffer), an overflowing range, a
  truncated frame, trailing bytes after a frame, a wrong `decoded_len`, a `len` above the
  cap (and `AllocTag::Io` total allocations unchanged), a permission failure, a directory
  opened as a file, a request after `shutdown`, a read on a `File` after the last `Io`
  dropped. `StorageFull` is produced on a size-limited filesystem where the runner has one
  (a `tmpfs` with `size=` on Linux); elsewhere the mapping of `ENOSPC` is `core`'s test.
- **Short reads.** A file shorter than `offset + len` by one byte is `InvalidData`; a file
  exactly `offset + len` long succeeds.
- **Decode.** A zstd frame round-trips to the bytes it was made from, in a buffer of
  exactly `decoded_len`; the content-size header disagreeing with `decoded_len` is
  `InvalidData`.
- **Atomic replace.** The new content is seen whole after `Ok`; the previous content is
  kept and the temporary removed when the write fails (a `path` that is a directory, a
  read-only directory); the temporary is created in the same directory (observed by a
  listing taken while a slow replace is in flight); a `File` opened before a replace reads
  the previous content after it, on Unix and on Windows; a replace over a path we hold
  open ourselves succeeds on Windows. Durability across a power loss is
  `docs/verification.md`'s row, not a test.
- **Directories.** A listing is sorted, kinds are the entries' own, a symbolic link is
  `Other`, a `.tmp` is listed; `create_dir_all` creates the chain and is idempotent.
- **The pool.** Threads are created through a spy `Platform` with the names, class and
  indices stated; `pool_threads` requests run at once (a barrier each waits on with two
  threads); `AllocTag::current()` is `Io` on the pool thread; a request's buffer is
  charged to `Io`.
- **Shutdown.** Queued requests resolve `Cancelled`, started ones finish, `shutdown`
  returns after the threads exit; dropping the last `Io` with requests in flight neither
  hangs nor panics and the started requests complete.
- **Tracing.** A capturing subscriber sees one span per request with the names and fields
  of the table, `error` recorded on a failure and empty on success.
