# lactose-io

File IO (architecture section 4, `io`): whole files and positional ranges of a file read
into caller-owned buffers, a range decoded as part of the same request, the atomic replace
of section 15.3, and directory listing, each a `Send + 'static` future completed by a pool
of platform threads of class `ThreadClass::Io`. SILO reads packs through it, `app` writes
and reads saves through it, the cooker reads sources and writes outputs through it. The
simulation never calls it.

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside |
| Unsafe | none in the library, `#![forbid(unsafe_code)]`; one test's allocator wrapper in `tests/requests.rs`; `SAFETY.md` argues it and says where the completion backends' `unsafe` will live |

**Status: implemented for milestone 1**, by the blocking backend on every platform. Every
body runs, and every row of "Proof for milestone 1" names the tests that hold it. Every
sentence under "Behaviour the signatures cannot express" is one a dependent crate may rely
on. Architecture sections 3, 4, 5.1, 9.7, 14, 15.3, 19, 20, 21 and 22 and
`docs/verification.md` section 3 are the design; where this file and the architecture
disagree, the architecture wins and this file is wrong.
Decision D130 records the choices those sections left open. `crates/core/README.md`
(errors, `AllocTag`, `ThreadClass`) and `crates/task/README.md` (the executor, `Task`,
`Platform`, `block_on`, `yield_now`) are the contracts this crate builds on and never
restates.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `asset`, `persist`'s callers in `app`, `cook`
and `app` are written against this surface. `tests/sketch.rs` is a compile-only sketch of
what each of them writes: a change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Io` | The pool handle: `Clone`, one per process, started by `app` with `Io::start(platform, config)` and cloned into whatever submits requests; `open`, `read_file`, `replace`, `read_dir`, `create_dir_all` are the requests that take a path; `pool_threads`; `shutdown(self)` |
| `IoConfig` | The one setting, `pool_threads: NonZero<u32>`, `#[non_exhaustive]`; `IoConfig::new(pool_threads)` for a count `app` chose, `for_platform` and `for_topology` for section 9.7's rule, half the cores rounded up, at least one, the same on client and server |
| `File`, `FileId` | An open file: `Clone` over one descriptor, `path`, `size` at the open, `id` for the spans; `read_at(offset, len)` and `read_encoded_at(offset, len, encoding)` are the positional reads, any number in flight at once (one at a time per handle on Windows, below) |
| `Encoding` | How a stored range is decoded as part of the same request: `Zstd { decoded_len }`, the exact decoded length from the index that named the range; `#[non_exhaustive]` for a platform's own format |
| `Open`, `ReadAt`, `ReadFile`, `Replace`, `ReadDir`, `CreateDirAll` | The futures, one per request: `Send + Sync + Unpin + 'static`, submitted at the first poll, cancelled by drop; each resolves to all of what was asked or an error. `Replace` resolves to `Result<(), ReplaceFailed>`, every other to `lactose_core::Result` |
| `ReplaceFailed` | A failed replace: `error`, and `bytes`, the bytes it was given, handed back unchanged and uncopied for a retry; `#[non_exhaustive]`, converts into its `Error` with `?` |
| `DirEntry`, `EntryKind` | One entry of a listing: its own name and its own kind (`File`, `Directory`, `Other`), sorted by name |
| `MAX_READ_BYTES` | 1 GiB, the most one request may ask for, stored, decoded or whole-file; above it `ResourceExhausted` before any allocation |

Everything else is hidden and changeable: the `Backend` trait (architecture section 20:
"its backends sit behind a trait inside the crate"), the blocking pool behind it, the
request enum, the completion a future and its backend share, the decode an encoded read
runs, the span constructors.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| SILO (`asset`) | A pack opened once for the session and read in ranges named by its index; each range into a buffer the store owns, as a `'static` future it spawns per load; the range decoded as part of the same request so a console's hardware decoder serves it unchanged (`docs/verification.md` section 3), and decoded on a background worker where software does it (architecture section 5.1); many reads in flight on one handle; a load abandoned by dropping its task; a corrupt index entry failing as a value, never as an allocation | `Io::open`, `File::read_at`, `File::read_encoded_at`, `ReadAt` (`Send + 'static`, cancelled by drop, decoding in its own polls), `MAX_READ_BYTES`; the loader runs over the returned bytes on the worker that awaited them |
| `app`, saves (SKIM, CARTON) | The bytes `Container::assemble` produced written as one request that is atomic and durable or has not happened, with failure modes `app` chooses policy from by kind: disk full, missing directory, permissions; a retry without a second copy of the bytes; a newer save never overtaken by an older one; the preamble then the sections as positional reads of one open slot; the slot listing; the save directory created at startup | `Io::replace`, its order per path, `ReplaceFailed` and the error table, `Io::open` + `File::read_at`, `Io::read_dir`, `Io::create_dir_all` |
| The cooker (RICOTTA) | Sources listed and read, outputs written whole, from a tool with no scheduler of its own | The same futures through `Executor::block_on`; there is no second, blocking API |
| The server (section 9.7) | A pool sized to half the cores, separate from the background workers, so a region publish never waits behind another; threads from the platform, not `std::thread` | `IoConfig::for_platform`, `Io::start` through `Platform::spawn_thread` with `ThreadClass::Io`, `pool_threads` requests run at once |
| Profiling (section 3) | One span per request, target `io`, with the fields a profiler joins on | The span table below |
| The executor (WHISK) | Nothing of it is used but the platform for threads; a request's future is a plain `std` future woken through its `Waker`, so any poll loop drives it | `Io::start(&dyn Platform, ..)`; the futures are `Unpin` |

## Behaviour the signatures cannot express

### Requests and futures

- Every request is a future, and nothing is submitted until it is first polled: no queue
  entry, no system call, no span. A future created and dropped unpolled has cost its own
  allocation, and nothing else.
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
  rename or its cleanup. An encoded read dropped after the pool handed it its stored bytes
  frees its buffers on the thread that drops it. The drop itself never blocks and never
  waits for the pool.
- One future is one request, and a future awaited to its end before the next is created
  or polled gives one request in flight. Several requests in flight at once are several
  futures polled at once: one `Task` per request through `Executor::spawn`, or any poll
  loop that polls them together. A loop that awaits them one after another runs them one
  after another.
- Requests start in the order their futures were first polled (one FIFO queue) and
  complete in no particular order, with one exception: a replace starts only after every
  replace of the same path first polled before it has finished (below), and one that
  waited for them joins the back of the queue when the last of them finishes or is
  skipped, so it starts after every request first polled while it waited. Otherwise the
  one ordering guarantee between requests is through the caller: a request's effects are
  visible to every request whose future is first polled after the first request's future
  resolved. Other requests in flight at once on the same path are unordered, with each
  other and with a replace of it: a `read_file` of a path while a replace of it is in
  flight sees the previous content or the new.
- There is no priority. `LoadPriority` in `asset-api` orders what SILO submits, not what
  this crate runs; the IO budget of milestone 3 decides whether that changes.
- Each request's own bookkeeping (the future, the request record, the completion) is
  allocated on the caller's thread under the caller's `AllocTag`. Every buffer a request
  returns is allocated on the pool thread under `AllocTag::Io`; freeing it anywhere is
  charged back to `Io`.

### Reads

- `File::read_at(offset, len)` resolves to exactly `len` bytes read from `offset`, in a
  `Vec<u8>` whose length and capacity are `len`, owned by the caller. `offset + len`
  overflowing `u64` is `InvalidData` and `len` above `MAX_READ_BYTES` is
  `ResourceExhausted`, decided before anything is submitted. On the pool thread the range
  is checked against the file's current size before the buffer is allocated: a file
  ending before `offset + len` is `InvalidData`, and so is a file that shrinks under the
  read, with no partial buffer. A buffer the allocator refuses is `ResourceExhausted`,
  never an abort. A `len` of zero resolves to an empty buffer without a system call,
  whatever the offset.
- `File::read_encoded_at(offset, len, encoding)` resolves to the `len` stored bytes at
  `offset` decoded, in a `Vec<u8>` whose length and capacity are `encoding`'s
  `decoded_len`. It is one request at the interface (`docs/verification.md` section 3):
  - The pool reads the stored bytes, with the checks of `read_at`, and checks them before
    the decoded buffer is allocated. They must be exactly one frame of `encoding`,
    starting at `offset` and ending at `offset + len`, and not a skippable frame; the
    frame's own content-size header must be present and equal `decoded_len`. A truncated
    frame, trailing bytes after it (a second frame included), a skippable frame, or a
    missing or disagreeing content size is `InvalidData`, and so is an empty range,
    decided before anything is submitted. The cooker therefore writes every frame with its
    content size (zstd's one-shot compression does; a streaming one must pledge the size).
    The pool then allocates the decoded buffer under `AllocTag::Io` (`ResourceExhausted`
    if the allocator refuses) and hands both buffers to the future.
  - The decode runs in the `ReadAt` future's own poll, on the thread that polls it, never
    on a pool thread: architecture section 5.1 gives asset decode to the background
    workers, and SILO's loads run there. It streams the frame in slices: a poll decodes
    for at most about 0.5 ms, then wakes its own waker and returns `Pending`, as section
    5.1's background CPU work yields, so a 32 MiB range is many polls and never one long
    one. Within a slice the decoder is given at most 64 KiB of output and at most 64 KiB of
    the stored range at a time, never the whole of either: with room for the whole content
    and the whole frame in hand, zstd decodes the frame in one call however long it is,
    and with the whole frame in hand it consumes in one call every block that writes
    nothing or little (a crafted frame of millions of empty blocks, or of one-byte
    blocks), however many there are. Each slice runs inside the request's span.
  - Decoding in steps makes zstd hold a window buffer of its own for the decode, up to
    the smaller of the frame's window and `decoded_len`, and at most 2^27 bytes. zstd
    allocates it, and its context, with its own `malloc`, outside the global allocator:
    neither is charged to `AllocTag::Io`, neither is one of this crate's fallible
    allocations, and both are freed when the decode ends or the future is dropped. The
    decoder's window limit is set to 2^27 explicitly, so the bound is this crate's and
    not zstd's default: a frame whose window exceeds 2^27 bytes is `InvalidData`, and the
    cooker never compresses with a window log above 27.
  - zstd failing to allocate its own memory is `ResourceExhausted`. Every other error the
    decoder reports is `InvalidData`, and so is a frame that decodes to any length but
    `decoded_len` or ends before every stored byte is consumed. The stored bytes are never
    handed out; their buffer is freed when the decode ends.
  - `decoded_len` is the caller's claim from the index that named the range, checked
    against the frame's header and trusted for the allocation; the frame is never allowed
    to grow the buffer past it. `len` or `decoded_len` above `MAX_READ_BYTES` is
    `ResourceExhausted` before anything is submitted.
  - A backend with a hardware decoder completes the whole request itself, decode
    included; the future then resolves without decoding.
- `Io::read_file(path)` resolves to the file's whole content. On the pool thread the
  file is opened and its size taken; a size above `MAX_READ_BYTES` or a buffer the
  allocator refuses is `ResourceExhausted` before anything is read. Exactly that many
  bytes are then read, and one more is tried: a file that shrank or grew while it was read
  is `InvalidData`, with no buffer, never a truncated content or one past the cap. A
  caller reading a file another process is writing retries. It is the one-shot form; a
  file read in ranges is opened once.
- A `File` is read-only, positional and cursorless: reads on it share nothing but the
  descriptor, and any number may be in flight at once, on any threads. `File::size()` is
  the size at the open; a read checks its range against the file, not against `size()`.
- On Windows the blocking backend reads through `std`'s synchronous handles, and the
  system serialises every operation on one synchronous handle: reads on one `File` run one
  at a time there, whatever `pool_threads` is. Reads on different `File`s, two opens of
  one path among them, run at once. `docs/verification.md` has the row that measures it
  for SILO's packs.
- A `File` stays open while any clone or any read in flight on it exists, and closes when
  the last is dropped, on that thread, without a request. A file replaced while a handle
  on it is open keeps serving the content the handle was opened on: the previous content
  on Unix, where the rename unlinks the name and not the inode, and on Windows, where
  `std` opens with delete sharing so the rename succeeds and the handle keeps the old
  file. Packs are immutable and saves are only ever replaced, so no caller sees a file
  change under a handle.
- A read on a `File` whose pool has stopped is `Cancelled`, except a `read_at` of `len`
  0, which resolves to an empty buffer without reaching the pool.
- `MAX_READ_BYTES` caps each request, not the pool: requests in flight at once may each
  hold that much, and an encoded read holds its stored and its decoded buffer until its
  decode ends. While it decodes, an encoded read also holds zstd's window buffer, up to
  the smaller of the frame's window and `decoded_len` and at most 2^27 bytes, which zstd
  allocates outside the global allocator and so is not charged to `AllocTag::Io`.

### The atomic replace

- `Io::replace(path, bytes)` is architecture section 15.3's operation as one request. On
  the pool thread: on the first replace into its directory, the temporaries an earlier
  process left there are removed (below); a temporary file `<name>.<unique>.tmp` is
  created in the same directory as `path` (so the rename is never across volumes) with
  `create_new`, so a leftover of another attempt is never reused, the next unique name
  being tried, a bounded number of times, when one exists already; `bytes` is written to
  it in full; the temporary is synced (`fsync`, which `std` issues as `F_FULLFSYNC` on
  Apple platforms, and `FlushFileBuffers` on Windows); it is renamed over `path`; and on
  Unix the directory is synced so the rename itself is durable. A `path` with no
  directory part (`slot0.carton`) is in the current directory, which is the one synced.
- When the future resolves `Ok`, a reader of `path` on this machine sees all of `bytes`,
  and a crash or power loss after that point keeps them, to the extent the platform's
  rename is durable (`docs/verification.md` has the row). At no point does a reader of
  `path` see anything but the previous content or the new: there is no moment at which
  `path` is missing or partial.
- Replaces of one path run in the order they were submitted (their futures first
  polled), dropped or not: a replace of `path` starts only after every replace of the same
  `path` submitted before it has finished, so the last submitted is the last to land, and
  a newer save is never overtaken by an older one whose future was dropped. A replace
  waiting behind another of its path holds no pool thread, and no pool thread ever waits
  for one; when the one before it finishes, it joins the back of the queue, behind every
  request first polled while it waited. A replace dropped before it started never runs:
  the next of its path waits for it only until a pool thread reaches it in the queue and
  skips it. Paths are compared as `Path` values, as given, within one `Io`: two spellings
  of one file (`saves/a` and `./saves/a`) are two paths, and replaces through two `Io`s
  share no order. Such replaces are unordered with each other: each runs every step with
  its own temporary, the last rename wins, and neither fails because of the other.
- A replace whose `path` ends in a symbolic link renames over the link: `path` becomes a
  regular file holding `bytes`, and the link's target is left unchanged. A symbolic link
  among the directories of `path` is followed, and the temporary is created in the
  directory it leads to.
- A failure resolves `Err(ReplaceFailed)`, which holds the error and `bytes`, unchanged and
  never copied, so a caller retries without keeping a copy of its own. That holds for
  every failure, `Cancelled` included.
- A failure before the rename keeps `path`'s previous content, and the temporary is
  removed (best effort: a crash between the failure and the removal can leave it). A
  failure syncing the directory after the rename resolves `Io` with the new content in
  place and its durability unknown; writing the same bytes again is the remedy. Every
  failure after the rename is `Io`, so `NotFound` and `StorageFull` always mean `path` is
  untouched. The kinds: `StorageFull` when the disk or quota fills during the creation,
  the write or the sync; `NotFound` when `path`'s directory does not exist, which callers
  prevent with `Io::create_dir_all` at startup rather than retry; `Io` for a permission
  failure, for a `path` that is a directory or names no file, on Windows for a `path`
  another process holds open without delete sharing, and for any failure after the
  rename. A cross-volume rename cannot happen.
- A `Replace` dropped before it started never touches the disk. One dropped after it
  started runs every step, so the path is replaced or left as it was and the temporary is
  cleaned up; the drop only discards the result.
- A temporary's `<unique>` is sixteen lower-case hexadecimal digits: eight of a token
  taken once per process (hashed by `std`'s randomly keyed `RandomState` from the process
  id and the time it is first needed), then eight of a process-wide counter. The first
  replace into a directory through an `Io`, keyed as the directory is spelled, sweeps it
  before creating its temporary: it removes every file of the directory named
  `<any>.<unique>.tmp`, of any name, whose token is not this process's. These are what a
  crash, or a process exit that skipped a replace's cleanup, left. Later replaces into
  the directory through that `Io` do not sweep it; a directory spelled two ways is swept
  twice, which costs one more listing. The sweep is best effort: a removal that fails
  does not fail the replace. It never removes a temporary of this process, so no replace
  removes another's, whatever the spellings and whichever `Io`. Two processes writing
  into one directory are unsupported: one's sweep may remove the other's temporary while
  it is written, failing that replace.
- A `.tmp` is an ordinary file to `Io::read_dir` until a later process's first replace
  into its directory removes it, and one exists beside every replace in flight. Callers
  that list a directory of slots ignore names ending in `.tmp`.

### Directories

- `Io::read_dir(path)` resolves to every entry of the directory, excluding `.` and `..`,
  sorted by `name` as `OsString` orders (byte order of the name), each with the kind of
  the entry itself: a symbolic link is `EntryKind::Other` whatever it points to, and so is
  anything that is neither a regular file nor a directory. `NotFound` if there is no such
  directory; `Io` if `path` is not a directory or cannot be read. A directory changed
  while it is being listed gives some listing of it; the sort is of what was read.
- `Io::create_dir_all(path)` creates every missing directory on the way to `path` and
  resolves `Ok` when `path` is a directory, including when it already was. `Io` on a
  permission failure, when a file is in the way, or when a directory on the way is removed
  while it runs; `StorageFull` when the disk or quota is full.

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
  first system call to its result. `pool_threads` is therefore the number of requests the
  pool runs at once; a request waits for a thread, and a replace for the earlier replaces
  of its path, never for anything else. A pool thread makes file system calls and checks
  frames; it decodes nothing.
- `IoConfig::for_topology` is `(performance + efficiency).div_ceil(2).max(1)`: four
  threads on a 4+4 part, eight on a server the topology reports as fifteen homogeneous
  cores, one on a four-vCPU runner. Client and server use the same rule until a
  measurement says otherwise (`docs/verification.md`); `app` may set `pool_threads`
  directly with `IoConfig::new`.
- `Io` is `Clone`, and every clone is the one pool. `Io::shutdown(self)` stops it:
  requests already started finish; requests queued or waiting behind a replace of their
  path, and requests submitted after it from any clone, resolve `Cancelled`; then it
  blocks until every thread has exited, in every clone that calls it at once. It is for
  the main thread and tools, before `Executor::shutdown`. Never call it from a pool thread,
  which runs the wakers of the requests it completes: behind a `shutdown` already joining
  that thread it hangs, and alone it joins the thread it runs on, which panics on Unix (and
  so aborts a binary) and hangs on Windows. Clones left after it are inert handles whose
  requests resolve `Cancelled`. An encoded read whose stored bytes arrived before the stop
  still decodes.
- Dropping the last `Io` clone without `shutdown` stops the pool the same way but does not
  wait: the threads exit after their current request and are not joined. Open `File`s
  outlive it as handles whose reads resolve `Cancelled`.
- No request runs its system calls on the caller's thread, blocks the caller's thread, or
  waits for the executor; the one work done in a future's poll is an encoded read's decode,
  in 0.5 ms slices. The pool never calls into the executor; a request's future is woken
  through the `Waker` of its last poll, so `Executor::block_on` and a hand-written poll
  loop drive it as well as a spawned task does.

### Errors

Every failure is a `lactose_core::Error` whose kind is the one policy dispatches on, a
failed replace's inside its `ReplaceFailed`. A system call's `std::io::Error` maps through
`From<std::io::Error>` and keeps its source, except that the two kinds no request returns
are made `Io`: `Unsupported` (`ENOTSUP`, from an `F_FULLFSYNC` on a file system without
it) and `Network` (a timeout or a reset on a network file system). A zstd error is
mapped from its code, never through `From`: `ResourceExhausted` when zstd cannot allocate
its own memory, `InvalidData` for every other.

| Kind | When |
|---|---|
| `NotFound` | `open`, `read_file`: no such file. `read_dir`: no such directory. `replace`: the directory of `path` does not exist, before the rename only |
| `InvalidData` | A read whose range runs past the end of the file, overflows, or (encoded) is empty; a file that shrinks under a read or that changes size under `read_file`; a decode of anything but one whole, non-skippable frame whose header and output are exactly `decoded_len`; a frame whose window exceeds 2^27 bytes; any other error zstd reports |
| `ResourceExhausted` | A `len`, `decoded_len` or whole-file size above `MAX_READ_BYTES`, before any allocation; a buffer the allocator refuses; zstd unable to allocate its window or context during a decode; the operating system out of memory |
| `StorageFull` | `replace`: the disk or quota filled during the creation, the write or the sync, before the rename. `create_dir_all`: the disk or quota is full |
| `Io` | Anything else the operating system reports: permissions, a directory where a file was named or a file where a directory was, a Windows sharing violation, a replace's temporary name taken on every try, every failure of a replace after its rename |
| `Cancelled` | A request submitted to a pool that has stopped, or queued or waiting behind a replace when it stopped. Never from dropping the future: a dropped future has no one to tell |

The kinds each request can return, and no other:

| Request | Kinds |
|---|---|
| `Io::open` | `NotFound`, `ResourceExhausted`, `Io`, `Cancelled` |
| `File::read_at` | `InvalidData`, `ResourceExhausted`, `Io`, `Cancelled` |
| `File::read_encoded_at` | `InvalidData`, `ResourceExhausted`, `Io`, `Cancelled` |
| `Io::read_file` | `NotFound`, `InvalidData`, `ResourceExhausted`, `Io`, `Cancelled` |
| `Io::replace` | `NotFound`, `StorageFull`, `ResourceExhausted`, `Io`, `Cancelled` |
| `Io::read_dir` | `NotFound`, `ResourceExhausted`, `Io`, `Cancelled` |
| `Io::create_dir_all` | `StorageFull`, `ResourceExhausted`, `Io`, `Cancelled` |

`VersionMismatch`, `Network`, `DeviceLost`, `Unsupported` and `Platform` are never
returned by a request. `Io::start` returns the platform's own error, of whatever kind the
platform gives a thread it cannot create (`Io` from `StdPlatform`).

### Tracing

One span per request, target `io`, level `INFO`. The target is the crate's role name:
`io` has no codename, an exception to architecture section 19's "Log targets are the
codenames" that D130 states, as `physics` is for `physics-api`'s contract. The span is
created at the future's first poll, when the request is submitted, and closed when the
request's state is released (after the result is taken or the future is dropped). It is
entered on the pool thread for the work, and, for an encoded read, in each decode slice of
the future's poll. A profiler that shows entered spans only, as Tracy does, sees the work
and not the wait. A future dropped unpolled creates no span; one polled and dropped before
the pool started it closes its span without it ever being entered. `error` is recorded on
completion with the kind, for a failure only.

| Span | Fields at creation | Recorded at completion |
|---|---|---|
| `io.open` | `path`, `file` (the `FileId` the handle will carry) | `size`, `error` |
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
  blocking pool, with its one-read-at-a-time handles on Windows.
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
  deletes: saves are replaced, packs are immutable, the cooker writes over its outputs,
  and a replace sweeps the temporaries an earlier process left. A save-slot delete in
  the UI is the first consumer and brings `remove_file` with it.
- **Two processes writing into one directory.** A replace's sweep removes every
  temporary with another process's token, so it assumes no other process writes into the
  directory; a lock file or a per-process subdirectory waits for a consumer that runs two.
- **Metadata without opening** (size, modified time) and **watching a directory** for the
  cooker's hot reload (`asset`'s milestone 2 work). Both are requests of the same shape
  and arrive with their consumer.
- **A platform's own compression format** as a second `Encoding` variant, when a console
  backend brings a decoder for it.

## Proof for milestone 1

The tests, all under `cargo nextest`, none under SOUR: this crate is outside the
determinism boundary and nothing in it hashes. Every one asserts on counts and orderings,
never on timings; a bounded wait only turns a hang into a failure. The proofs that hold a
pool thread at a chosen point of a request do it through a `#[cfg(test)]` hook of the
blocking pool and are the unit tests of `src/tests.rs`; the rest go through the public
surface in `tests/`, with `tests/common/mod.rs` as their shared support. Counts of
`AllocTag::Io` and of the temporary counter assume one process per test, as nextest runs
them. Each row ends with the tests that hold it.

- **Config.** `for_topology` on the listed topologies (exists); `IoConfig::new` keeps its
  count. `src/config.rs`: `half_the_cores_rounded_up_at_least_one`, `new_keeps_its_count`.
- **Cancellation.** A future created and dropped unpolled submits nothing: the queue is
  untouched and no span is created. A future dropped while queued never runs: with one
  pool thread held by a slow request, a queued replace is dropped and the path is never
  created. A future dropped after its request started completes on the pool: no panic, the
  buffer is freed, `AllocTag::Io` live bytes return to zero. The drop returns at once
  while the pool thread is still inside the request. An encoded read dropped between two
  decode slices frees both buffers, and `AllocTag::Io` live bytes return to zero.
  `src/tests.rs`: `a_future_dropped_unpolled_submits_nothing`,
  `a_future_dropped_while_queued_never_runs`,
  `a_future_dropped_after_its_request_started_completes_on_the_pool`,
  `an_encoded_read_dropped_between_two_slices_frees_both_buffers`; the span half in
  `tests/spans.rs`.
- **Ordering.** With one pool thread, requests start in first-poll order. A replace
  awaited to `Ok` followed by a `read_file` of the same path sees the new content.
  `src/tests.rs`: `requests_start_in_first_poll_order`; `tests/requests.rs`:
  `a_replace_is_seen_whole_after_ok_and_by_the_next_read_file`.
- **Replace order per path** (the shape of the review's `review_replace_race.rs`). With
  two pool threads, a large replace A of a path is polled until its temporary exists
  (observed by a listing), and its future dropped; a small replace B of the same path is
  then submitted and awaited. B resolves `Ok` only after A has finished, the path holds
  B's bytes then, and still does once the pool is idle. A third replace of the same path
  dropped while it waits behind A never runs and does not delay B. Two replaces of
  different paths run at once. With one pool thread held, a replace waiting behind another
  of its path, and one waiting behind a replace dropped while queued, each start after the
  requests first polled while they waited. `src/tests.rs`:
  `replaces_of_one_path_run_in_submission_order_dropped_or_not`,
  `replaces_of_two_paths_run_at_once`,
  `a_replace_released_from_behind_another_of_its_path_joins_the_back_of_the_queue`.
- **Errors.** Each row of the table is produced: a missing file, a missing directory for
  a replace, a range past the end (and no partial buffer), an overflowing range, an empty
  encoded range, a truncated frame, trailing bytes after a frame, two concatenated frames
  whose sizes sum to `decoded_len`, a skippable frame before a frame, a frame without a
  content size, a content size above and below `decoded_len`, a `len` above the cap (and
  `AllocTag::Io` total allocations unchanged; for a `read_file`, no block of 1 MiB or more
  asked of the allocator), a permission failure, a directory opened as
  a file, a request after `shutdown`, a read on a `File` after the last `Io` dropped. Each
  decode failure is `InvalidData`, none `Io`. `StorageFull` is produced on a size-limited
  filesystem where the runner has one (a `tmpfs` with `size=` on Linux); elsewhere the
  mapping of `ENOSPC` is `core`'s test. The remapping of `Unsupported` and `Network` to
  `Io` is a unit test of the mapping function over constructed `std::io::Error`s.
  `tests/requests.rs`: `a_missing_file_or_directory_is_not_found`,
  `a_range_past_the_end_or_overflowing_is_invalid_data_and_allocates_nothing`,
  `every_stored_range_but_one_whole_frame_of_the_stated_size_is_invalid_data`,
  `a_length_above_the_cap_is_resource_exhausted_before_any_allocation`,
  `a_permission_failure_is_io` (Unix, skipped as root),
  `a_directory_opened_or_read_as_a_file_is_io`, `a_request_after_shutdown_is_cancelled`,
  `a_read_on_a_file_after_the_last_io_is_dropped_is_cancelled`; `tests/storage_full.rs`, run
  where `LACTOSE_IO_SMALL_FS` names a directory on a file system with less than 64 MiB
  free and passing without running elsewhere; `src/backend/blocking.rs`:
  `unsupported_and_network_become_io_and_keep_their_source`.
- **Short reads.** A file shorter than `offset + len` by one byte is `InvalidData`; a file
  exactly `offset + len` long succeeds. A `read_file` of a file appended to between its
  size check and its end (a test hook between the two) is `InvalidData`.
  `tests/requests.rs`:
  `a_file_one_byte_short_is_invalid_data_and_one_exactly_long_enough_is_read`;
  `src/backend/blocking.rs`: `a_file_appended_to_while_it_is_read_is_invalid_data`.
- **Decode.** A zstd frame round-trips to the bytes it was made from, in a buffer of
  exactly `decoded_len`; the content-size header disagreeing with `decoded_len` is
  `InvalidData`. A frame of several MiB, polled by hand, returns `Pending` with its own
  waker woken more than once before it resolves, and the decode runs on the polling
  thread, never the pool's. One step of a slice decodes at most 64 KiB of output and
  consumes at most 64 KiB of the stored range (a test hook counts both per
  `decompress_stream` call), so a whole frame given at once still takes several calls and
  polls, and so do a frame of 200 000 empty blocks and frames of one-byte blocks, raw and
  compressed. A frame of a little over 2^27 bytes of
  zeros compressed with `WindowLog(28)` is `InvalidData`; the same content compressed with
  `WindowLog(27)` decodes. zstd's memory-allocation code maps to `ResourceExhausted` and
  every other code to `InvalidData` (a unit test of the mapping). `tests/requests.rs`:
  `a_frame_round_trips_at_its_offset_into_a_buffer_of_exactly_decoded_len`,
  `every_stored_range_but_one_whole_frame_of_the_stated_size_is_invalid_data`,
  `a_frame_of_several_mib_decodes_in_many_polls_on_the_polling_thread`,
  `a_window_above_two_to_the_27_is_invalid_data_and_one_at_it_decodes`; `src/decode.rs`:
  `a_step_never_writes_more_than_its_share_of_output`,
  `a_step_never_consumes_more_than_its_share_of_a_frame_of_empty_blocks`,
  `a_step_never_consumes_more_than_its_share_of_a_frame_of_one_byte_blocks`,
  `zstd_allocation_failure_is_resource_exhausted_and_every_other_code_invalid_data`.
- **Atomic replace.** The new content is seen whole after `Ok`; the previous content is
  kept and the temporary removed when the write fails (a `path` that is a directory, a
  read-only directory), and the `ReplaceFailed` holds the very bytes given (same pointer
  and length); the temporary is created in the same directory (observed by a listing
  taken while a slow replace is in flight); a replace of a bare file name works in the
  current directory; the first replace into a directory removes a leftover
  `<any>.<sixteen hex>.tmp` whose token is not this process's, of any name, and leaves a
  temporary with this process's token, a file `<name>.bak.tmp` and a file with fifteen
  hex digits; a second replace into the directory does not list it again (a leftover
  placed after the first is still there); with two `Io`s, and with two spellings of one
  path (`d/a` and `./d/a`), replaces of one path held in flight at once both resolve
  `Ok`, the path holds one of the two contents, and no temporary is left; a temporary
  name already taken is skipped to the next; a `File` opened before a replace
  reads the previous content after it, on Unix and on Windows; a replace over a path we
  hold open ourselves succeeds on Windows; a replace of a symbolic link replaces the link
  and leaves its target (Unix). Through the pool, the steps come in order: the temporary
  is created empty, written, then synced while the path still holds the previous content;
  the directory is synced after the rename, with the path holding the new content (the
  sync functions themselves tell a test hook). A failure after the rename is `Io` (forced
  with a test hook in place of the directory sync). Durability across a power loss is
  `docs/verification.md`'s row, not a test. `tests/requests.rs`:
  `a_replace_is_seen_whole_after_ok_and_by_the_next_read_file`,
  `a_replace_of_a_path_that_is_a_directory_fails_with_its_bytes_and_leaves_nothing`,
  `a_permission_failure_is_io`, `a_replace_of_a_bare_name_lands_in_the_current_directory`,
  `a_file_opened_before_a_replace_reads_the_previous_content_after_it` (a handle of our own
  held open across it),
  `a_replace_of_a_symbolic_link_replaces_the_link_and_leaves_its_target`; `src/tests.rs`:
  `the_temporary_is_created_beside_the_path`,
  `a_replace_writes_and_syncs_its_temporary_then_renames_then_syncs_the_directory`,
  `the_first_replace_into_a_directory_sweeps_only_foreign_temporaries`,
  `replaces_through_two_pools_or_two_spellings_both_land`,
  `a_temporary_name_already_taken_is_skipped`; `src/backend/blocking.rs`:
  `the_temporary_pattern_is_any_name_sixteen_hex_digits_and_tmp`,
  `a_failure_after_the_rename_is_io_with_the_new_content_in_place`.
- **Directories.** A listing is sorted, kinds are the entries' own, a symbolic link is
  `Other`, a `.tmp` is listed; `create_dir_all` creates the chain and is idempotent.
  `tests/requests.rs`: `a_listing_is_sorted_by_name_with_each_entry_s_own_kind` (the link on
  Unix), `create_dir_all_creates_the_chain_and_succeeds_again`.
- **The pool.** Threads are created through a spy `Platform` with the names, class and
  indices stated; `pool_threads` requests run at once (a barrier each waits on with two
  threads); `AllocTag::current()` is `Io` on the pool thread; a request's buffer is
  charged to `Io`. `src/tests.rs`:
  `the_threads_come_from_the_platform_with_their_names_class_and_indices`,
  `a_thread_the_platform_refuses_fails_the_start_after_stopping_the_others`,
  `pool_threads_requests_run_at_once_under_the_io_tag`; `tests/requests.rs`:
  `reads_on_one_file_in_flight_at_once_each_get_their_own_range`.
- **Shutdown.** Queued requests and replaces waiting behind their path resolve
  `Cancelled`, started ones finish, `shutdown` returns after the threads exit; a second
  `shutdown` from a clone, called while the first joins a thread held in a request, does
  not return before that thread has exited; dropping the last `Io` with requests in
  flight neither hangs nor panics and the started requests complete. `src/tests.rs`:
  `shutdown_cancels_what_waits_finishes_what_started_and_joins`,
  `every_shutdown_called_at_once_returns_after_every_thread_has_exited`,
  `dropping_the_last_handle_stops_the_pool_without_waiting`; `tests/requests.rs`:
  `an_encoded_read_whose_stored_bytes_arrived_before_the_stop_still_decodes`.
- **Tracing.** A capturing subscriber sees one span per polled request with the names and
  fields of the table, `io.open` carrying `file` from its creation and `size` on success,
  `error` recorded on a failure and empty on success, and no span for a future dropped
  unpolled. `tests/spans.rs`: `one_span_per_polled_request_with_the_fields_of_the_table`.
