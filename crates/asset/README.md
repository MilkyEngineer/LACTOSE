# SILO (`lactose-asset`)

The asset store (architecture section 4, `asset`; section 14): cooked packs mounted and
read through `lactose-io`, one load per asset version on the executor's background
workers, versioned publication and hot reload behind the two store traits of
`lactose-asset-api`, the pack format RICOTTA writes, and the region baseline as an asset.
`app` builds one store per process and hands it on as `dyn` resources; nothing else names
this crate but the cooker, for the pack writer, and from milestone 3 `persist`, for the
baseline.

| | |
|---|---|
| Tier | backend; it names foundation crates only (`asset-api`, `bundle`, `core`, `io`, `task`) |
| Determinism boundary | outside. A boundary world reaches it only as `dyn PublishedAssets` and through the `PinnedTable` pinned from it (architecture section 8.2); no boundary crate may depend on it |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** Every public signature is final, and the crate does what this
file says, proved by the tests under "Proof for milestone 1". Every sentence under "The
pack format" and "Behaviour the signatures cannot express" is one a dependent crate may
rely on. Architecture sections 3, 4, 5.1, 8.2, 9.7, 14, 15.2, 18, 19, 20, 21 and 22 are
the design; where this file and the architecture disagree, the architecture wins and this
file is wrong. Decision D136 records the choices those sections left open.
`crates/asset-api/README.md` (the two traits, versions, handles, the pinned table, the
conformance suite), `crates/io/README.md` (requests, ranges, the decode, the atomic
replace), `crates/task/README.md` (tasks, `block_on`, shutdown), `crates/bundle/README.md`
(the baseline) and `crates/core/README.md` (errors, `AllocTag`) are the contracts this
crate builds on and never restates.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `app`, the cooker and `persist` are written
against this surface. `tests/sketch.rs` is a compile-only sketch of what each of them
writes: a change that breaks it is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `Store` | The store handle: `Clone`, one store per process; `Store::new(executor, io, loaders)`; `mount(path)` for a cooked pack; `settled()` for the one wait; it is a `lactose_asset_api::AssetCatalog` and so a `PublishedAssets`, and a clone coerces to `Arc<dyn AssetCatalog>` and from there to `Arc<dyn PublishedAssets>` |
| `Mount`, `Settled` | The two futures: `Send + Unpin + 'static`, lazy, cancelled by drop. `Mount` resolves to `lactose_core::Result<()>`, `Settled` to `()` |
| `PackBuilder`, `Storage` | The pack writer, for the cooker and for tests: `add(id, &cooked, storage)` returns the version it recorded, `finish()` returns the pack's bytes; `Storage::Raw` or `Storage::Zstd { level }` per payload |
| `PACK_FORMAT_VERSION` | 1: the pack format this build writes and mounts |
| `RegionBaseline`, `RegionBaselineLoader` | A region's cooked baseline as an asset kind, named by its content hash: `name(content_hash)` and `id(content_hash)`, `cook(&baseline)` for the cooker, `lookup(catalog, content_hash)` for `app` and `persist`, `baseline()` for the shared `Arc<Baseline>`; and its loader, which every store registers itself |
| `LOG_TARGET` | `"silo"`: the `tracing` target of everything this crate emits |

Hidden and changeable: the index decode a mount runs and the entry it yields
(`pack::Entry`), everything inside `Store` (the packs, the loads, the published versions,
the lock), and what `Mount` and `Settled` hold.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Where |
|---|---|---|
| `app`, building a runtime | One store over the cooked packs, built from the executor and the pool it already has and from every kind's loader; packs mounted in an order it controls, one after another or at once; the store dropped before the pool and the executor stop, with no shutdown of its own to call | `Store::new`, `Store::mount` (the place is the call's), "Settling, cancellation and shutdown" |
| `app`, wiring resources (architecture section 4) | The catalogue for everything outside the boundary and the exact-version view for a boundary world, from the one store | `Arc::new(store.clone())` as `Arc<dyn AssetCatalog>`, upcast to `Arc<dyn PublishedAssets>`; pinned in `lib.rs` |
| `app` at milestone 1 | Nothing: `app` keeps its `Cooked` stand-in (D131). A binary that wants cooked packs behind it, for the edit-to-playing measurement, bridges the three calls over a store it loaded before the first step | `tests/sketch.rs` section 2: `mount`, `request_newest`, `settled`, then `RegionBaseline::lookup`, `resolve`, `PinnedTable::pin`, each answering at once |
| `app` at milestone 3, the store's driver | For each `RegionRequest::Load`: the baseline by content hash; the region's set requested; one consistent table once the set is published; the table pinned; all without blocking a frame and without resolving every region every frame; a later table when a pack was mounted; a failure as a value for `TickInput::failures` | `RegionBaseline::lookup`, `request_newest`, `generation`, `resolve`, `PinnedTable::pin`; "Publication" and "Hot reload" below; `tests/sketch.rs` section 3. The driver's side of it: it requests a set once, and retries a failure on a decision, never every frame, because a request that fails again moves the generation and would make it resolve every region every frame ("Publication") |
| A boundary world (architecture section 8.2) | Exact versions only; reads that touch no store, no lock and no atomic | `dyn PublishedAssets`; `PinnedTable`, which owns its assets and outlives the store |
| Code outside the boundary (the renderer and presentation, milestone 2) | The newest version of an asset by typed reference; to see a reload by asking again, and to know when asking again can matter | `dyn AssetCatalog::newest_typed`, `generation` |
| A client or a replay (milestone 4) | A table it was given, loaded by exact version, whichever pack holds each version, however many newer packs are mounted | `request(id, version)` searches every mounted pack; `PinnedTable::pin` |
| RICOTTA (`tools/cook`), the milestone 1 skeleton | A writer for the file the store mounts, so that the cooker states no frame rule and no index layout; the version each asset was recorded under; a repeated ID reported; the baseline's name, ID and bytes, the name for its collision report over every name it publishes; a way for its output to reach a running store | `PackBuilder`, `Storage`, `RegionBaseline::name` and `RegionBaseline::cook`; the bytes written whole with `Io::replace`; "The pack format" and "Mounting" below. The cooker is a leaf crate and may name this one |
| The dev loop (architecture section 18) | A newly cooked pack visible in a running store without a restart and without the caller listing what changed; a restart that reads no payload it does not need | `Store::mount`: the store reloads what it follows, and a mount reads a pack's tail only |
| `persist` at milestone 3 | The baseline a region's records were made against, by the hash the records carry, from the store it is handed | `RegionBaseline::lookup(catalog, records.baseline_hash)`; `RegionBaselineLoader` for its tests over the reference store |
| Tests and tools | A wait that ends when what was asked for is in, with no polling loop; the conformance suite | `Store::settled`; `tests/conformance.rs` |
| Profiling (architecture section 3) | Spans a profiler joins to `io`'s | "Tracing" below |

**What milestone 1 delivers, and what `Cooked` still covers.** Milestone 1 delivers the
store behind the two traits with the conformance suite passing, the pack format and its
writer, mounting and the hot reload it starts, the wait, the region-baseline kind, and
the tests of "Proof for milestone 1". Nothing in `app` names this crate at milestone 1:
`lactose_app::Cooked` keeps covering all three of its calls (a baseline by content hash,
a region's table, the pinning of a table) for `load`, `build_server`, `build_client`,
`Headless::start` and the client's fold, over whatever the binary puts behind it. That
may be this store, loaded before the first step, since a binary is a leaf crate; it need
not be. At milestone 3 `app` drops `Cooked` for the driver of `tests/sketch.rs`
section 3, which is when a load can arrive during a run, a table can change under a
loaded region, and the lifecycle's `Load`, `Cancel` and `Release` meet the store.

## The pack format

A pack is one immutable file: a payload area, an index, a 24-byte trailer. Every integer
is little-endian. `lactose-bundle` fixes baselines and overlays and no pack; this format
is this crate's (D136), written by `PackBuilder` alone and read by `Store::mount` alone.

| Part | Length | Contents |
|---|---|---|
| payload area | the rest | From offset 0 to the index: the stored bytes of the entries, each at the range its entry names |
| index | N | `u32` entry count, then the entries, strictly ascending by asset ID |
| entry | 57 + 8 per dependency | `u64` asset ID; `u64` version; `u64` asset kind; `u32` cooked format version; `u32` dependency count; each dependency as a `u64`, strictly ascending; `u8` storage, 0 for raw and 1 for zstd; `u64` offset; `u64` stored length; `u64` decoded length |
| trailer | 24 | `u64` N, the index's length; `u64` xxh64, seed zero, of the N index bytes; the magic `LPAK`; `u16` format version, 1; `u16` zero |

- A pack holds at most one version of an asset. The ID, version, kind, cooked format
  version and dependencies of an entry are the asset's `AssetId`, `AssetVersion` and
  `CookedHeader` (`crates/asset-api/README.md`): the index is where a store learns an
  asset's kind and dependencies before it reads a byte of it.
- The last eight bytes (the magic, the version, the zero) are the same in every format
  version, and a mount checks the magic and the version before anything else: a file
  without the magic is `InvalidData`, a pack of another version is `VersionMismatch`,
  whatever else it holds.
- A mount then requires, and refuses the whole pack with `InvalidData` otherwise: the
  file is at least 24 bytes; the reserved `u16` is zero; N is at most the file's length
  less 24, the index being the N bytes before the trailer and the payload area
  everything before it; the index hashes to the trailer's hash; the index is exactly its
  count and that many entries, with no byte left over, every count checked against the
  bytes that remain before anything is allocated or looped over; asset IDs are non-zero
  and strictly ascending; versions are non-zero; an entry's dependencies are non-zero and
  strictly ascending; the storage code is 0 or 1; `offset + stored length` does not
  overflow and ends inside the payload area; both lengths are at most
  `lactose_io::MAX_READ_BYTES`; a raw entry's decoded length equals its stored length; a
  zstd entry's stored length is at least one.
- A mount does not read, decode or check a payload, does not require the ranges to be
  disjoint or to cover the payload area (a later writer may share or pad), and does not
  check that an entry's content hashes to its version. A zstd range is checked to be one
  frame of its decoded length by `io` when it is read, and content against its version
  by `ErasedLoader::load` when it is loaded. The index's hash detects damage to the
  index; a version detects damage to its payload. Neither is a signature. The format
  forbids no dependency either: an asset that declares itself, or a cycle across packs,
  is a failed load ("Requests and loads"), not a malformed pack.
- An index is read with one request, so one longer than `MAX_READ_BYTES` (1 GiB) is
  `ResourceExhausted`. N is checked against the file first, from the trailer and the
  file's size, before any read of the index: an N past the file is `InvalidData` whatever
  its size, also above 1 GiB, and only an N that lies within the file and is longer than
  `MAX_READ_BYTES` is `ResourceExhausted`. Decoding an index allocates at most four
  times its length on a 64-bit host, so a well-formed index of the largest length costs
  a mount about 5 GiB: a pack is a trusted local file, not a message. An index entry is
  57 bytes and eight more per dependency: a pack of 100,000 assets without dependencies
  has an index of 5.7 MB.
- `PackBuilder::add` refuses `AssetId::NONE` and an ID already added with another
  version (`InvalidData`: two names that hash to one ID surface here, for the cooker's
  collision check to report), and content longer than `MAX_READ_BYTES`
  (`ResourceExhausted`); an ID added again with content of the same version adds nothing
  and returns the version, since equal content under one ID is one asset, as two regions
  with equal baselines are.
- What `PackBuilder` writes is a pack every mount accepts: payloads back to back from
  offset 0 in the order they were added, with no padding and no shared bytes; entries
  sorted by ID; under `Storage::Zstd`, exactly one zstd frame that carries its content
  size and whose window is at most 2^27 bytes (the three things `crates/io/README.md`
  requires of a stored range), kept only if it is shorter than the cooked bytes and
  stored raw otherwise, so an empty asset is always raw. The index records what was
  stored: no reader names a level.
- A pack's bytes are not an identity. A version hashes an asset's header and its decoded
  bytes (`CookedHeader::version_of`), so the same asset stored raw in one pack and
  compressed in another is one version, and nothing hashes a pack file.
- A pack is immutable once written: the cooker writes it whole with `Io::replace` and
  nothing appends to it. A file a store has mounted must not be written in place.
  Replacing it by rename is safe ("Mounting" below).

## Behaviour the signatures cannot express

### Words

- A pack is **mounted** from the moment its `Mount` resolves `Ok` until the last handle
  to the store is dropped. Each mount has a **place** in the store's mount order.
- The **named** version of an asset is the version in the mounted pack with the latest
  place among the mounted packs that hold the asset. This is what
  `crates/asset-api/README.md` calls the version the store's pack index names: SILO's
  index is its mounted packs taken together.
- A **load** is of one `(AssetId, AssetVersion)`; at most one is in flight per pair. It
  is **accepted** inside the call that starts it and **ends** exactly once, published or
  failed. It fails **for a dependency** when a declared dependency of it failed, and
  **for itself** otherwise: no pack holds it, its kind has no loader, or its read or its
  loader's call failed.
- A failure of a load for itself is **of the version** when the same content would fail
  the same way from any pack: no loader for its kind (`Unsupported`), a cooked format
  the loader does not read (`VersionMismatch`), and any failure `ErasedLoader::load`
  returned over bytes that hash to the version, which is the loader refusing intact
  bytes, whatever kind it gave. It is **of the source** when another pack that holds the
  version might load it: every failure of the read (`Io`; `ResourceExhausted`;
  `InvalidData` for a range past the file or a stored range that is not one frame of the
  stated length; `Cancelled`), and `InvalidData` from `ErasedLoader::load` when the
  bytes do not hash to the version. The store tells the last from a loader's refusal by
  hashing the content itself when the call returns `InvalidData`. `NotFound` is neither:
  no mounted pack holds the version.
- A version is **published** when its load ends well. A published version never changes,
  is never replaced and is never unloaded.
- The **current** version of an asset is the one `newest` answers with. A version
  becomes current when it is published while named, or when `request_newest` or a mount
  finds the named version already published. A current version is replaced only by
  another, never withdrawn.
- An asset is **followed** once its newest version has been asked for: by
  `request_newest`, or as a declared dependency of an accepted load. It stays followed
  until the last handle to the store is dropped.
- The **newest failure** of an asset is what `newest` answers with while the asset has no
  current version; "Lookups" below says when it is recorded.

### Building the store

- `Store::new(executor, io, loaders)` indexes the loaders by `AssetKind`, adds
  `RegionBaselineLoader`, and keeps the two handles. It spawns nothing, opens nothing and
  reads nothing. Two loaders of one kind are `InvalidData`, the message naming the kind:
  this is where kind names are collision-checked (`crates/asset-api/README.md`). The set
  of loaders is fixed for the store's life.

### Mounting

- `Store::mount(path)` opens the file with `Io::open`, reads its 24-byte trailer and
  then its index, each with one `File::read_at`, decodes and checks the index ("The pack
  format"), and mounts the pack. It costs one open and two reads, whatever the pack
  holds: its cost follows the index's length and never the payloads'. A file shorter
  than a trailer is refused after the open, with no read. The decoded index is allocated
  under `AllocTag::Asset` and kept while the pack is mounted.
- Each call of `Store::mount` takes the next place in the mount order when it is called,
  not when its future is polled or resolves (calls made at once from several threads
  take theirs in the order the store's lock grants them); the pack holds that place from
  the moment the future resolves `Ok`. A mount that fails, or whose future is dropped,
  leaves its place empty for good. So packs mounted by the same calls in the same order
  name the same versions once every mount has resolved, whatever order they resolved in,
  and a caller may await mounts one after another or spawn them all. While some are
  still in flight, the named versions are those of the packs mounted so far.
- A mount is atomic with respect to every other method: no call sees part of a pack.
  When it resolves, in one step: every version in the pack becomes requestable by exact
  version; the named versions change as the definition says; and the store does what
  `request_newest` does, at `LoadPriority::default()`, for
  1. every followed asset whose named version the mount changed, an asset it named for
     the first time included, unless that version is current or loading;
  2. every followed asset whose named version last failed for itself with a failure of
     the source, when the mounted pack holds that version and is now the pack the
     version is read from (the latest place that holds it); and
  3. every followed asset whose named version last failed for a dependency that this
     mount asks for again, under any of the three, and is not loading.

  The loads this accepts are accepted before the future resolves. Nothing else is asked
  for. A followed asset the mount did not rename, whose named version failed with a
  failure of the version, is left as it is, since the same content would fail the same
  way; so is one whose failure was of the source when the mount brings no new source
  for that version. A pack mounted over a damaged or unreadable one, holding the same
  versions, therefore repairs what the damage had failed, and the dependants with it,
  as a fresh process that mounted both would have loaded them from the later pack.
- Renaming changes no answer of `get`, `newest` or `resolve`. A mount changes an answer,
  and moves the generation, only through what those requests do: at the mount itself
  where one finds the newly named version already published, which becomes current, or
  accepts a load that ends inside the request (a kind without a loader and with no
  dependency to wait for); otherwise when the loads it accepted end.
- Mounting the same path again mounts the file that is at the path then, as another
  pack with a later place. A mounted pack keeps reading the file it opened: one replaced
  by rename (`Io::replace`) while mounted keeps serving its old content, as
  `crates/io/README.md` states for an open `File`, on Unix and on Windows. A file
  truncated or overwritten in place under a mount makes its loads fail as values
  (`InvalidData` from the read or from the version check); it is never undefined.
- A failed mount mounts nothing and changes nothing. The kinds:

  | Kind | When |
  |---|---|
  | `NotFound` | No file at `path` |
  | `InvalidData` | The file is not a well-formed pack by any rule of "The pack format"; the file's size changed under the reads |
  | `VersionMismatch` | A pack of a format version other than `PACK_FORMAT_VERSION` |
  | `ResourceExhausted` | An index that lies within the file and is longer than `MAX_READ_BYTES`; a buffer the allocator refuses |
  | `Io` | Anything else the open or a read reports: permissions, a directory at `path` |
  | `Cancelled` | The pool has stopped |

- Nothing is unmounted. Every mounted pack holds one descriptor and its decoded index
  until the last handle to the store is dropped, because an exact-version request may
  name any version any mounted pack holds.
- On Windows the blocking backend of `io` runs the reads on one `File` one at a time, so
  loads from one pack are read one at a time there, whatever the pool's size
  (`crates/io/README.md`; `docs/verification.md` has the row).

### Requests and loads

- `request` and `request_newest` never block, read nothing and run no loader. They take
  the store's lock, decide, register every load they accept and release the lock; only
  then do they spawn one task per accepted load that has something to read, with
  `Executor::spawn(WorkerClass::Background, ..)`, and store its `Task`, all before they
  return. The spawn is outside the lock because a load's future can be dropped inside
  `spawn`, on the calling thread, when the executor has shut down or is shutting down,
  and the guard that then records the load as `Cancelled` takes the lock. The same
  holds for the loads a mount accepts. They may be called from any thread, a worker
  included.
- `AssetId::NONE` names no asset and no pack can hold it: a request for it does nothing
  (nothing is followed, recorded or logged, and the generation does not move), and `get`
  and `newest` of it are `Failed` with `{ id: NONE, version: None, kind: NotFound }`,
  the failure `asset-api`'s typed lookups give an empty reference.
- `request(id, version, priority)`: if `(id, version)` is published or loading, nothing
  happens, the priority included. Otherwise a load of it is accepted, from the mounted
  pack with the latest place that holds exactly that version; no other pack is tried,
  also not after a failure. If no mounted pack holds it, the load ends inside the call,
  failed with `NotFound`. A recorded failure of `(id, version)` is retried this way. An
  exact request does not make the asset followed.
- `request_newest(id, priority)` makes `id` followed, then: if no mounted pack holds
  `id`, the load ends inside the call, failed with `NotFound` and no version, and
  `{ id, version: None, kind: NotFound }` becomes the asset's newest failure (every time
  it is called so); if the named version is published, that version becomes current if
  it is not; if the named version is loading, nothing happens; otherwise a load of the
  named version is accepted, a recorded failure of it retried.
- An accepted load whose pack entry was found goes through these steps; the last two say
  which failure it reports.
  1. **The dependencies.** Inside the accepting call, every declared dependency becomes
     followed, and each that was not followed before or has no current version is asked
     for as by `request_newest` at the load's priority, so the loads this accepts are
     accepted in the same call, transitively. A dependency that was followed already and
     has a current version is not asked for again: it is satisfied by that version,
     whatever became of the load of a newer one.
  2. **The loader.** If no loader is registered for the entry's kind, nothing is read and
     steps 3 and 4 are skipped.
  3. **The read.** On the task: `File::read_encoded_at` for a zstd entry, with the
     entry's decoded length, or `File::read_at` for a raw one, on the pack's `File`. The
     decode runs in the task's own polls in `io`'s 0.5 ms slices. The read does not wait
     for the dependencies.
  4. **The loader's call.** The task awaits `lactose_task::yield_now()` once, then calls
     `ErasedLoader::load(id, version, header, bytes)` with the entry's header, which
     verifies that the content hashes to the version and that the cooked format is the
     loader's, and decodes. The read's buffer is dropped when the call returns.
  5. **The end.** The load ends when steps 3 and 4, unless skipped, have ended and every
     declared dependency has a current version or has failed. A dependency has failed,
     for this purpose, when it has no current version and no load of its named version
     is in flight: its `newest` is then `Failed`, and its kind is that failure's. The
     load then fails with the first of: the kind of its first failed dependency in
     ascending `AssetId` order (it failed for that dependency); `Unsupported` if its
     kind has no loader; the read's failure; what `ErasedLoader::load` returned (in
     these three it failed for itself). Otherwise it is published.
  6. **Cycles.** A set of loads in which each waits for each other, directly or through
     loads of the set, is a cycle; a load whose asset declares itself, and so waits for
     itself, is a cycle of one. Its members can never be published, and none of them
     ends by step 5. They are decided together, in one step, once steps 3 and 4 of every
     member have ended and every dependency of a member outside the cycle has a current
     version or has failed. Each member then fails for its first failed dependency in
     ascending `AssetId` order, the other members counting as failed: with that
     dependency's kind if it is outside the cycle; with the kind decided for it if it is
     a member; and with `InvalidData` where following first failed dependencies from
     member to member comes back round. So a member whose lowest failed dependency is a
     missing asset outside the cycle is `NotFound`, a member whose lowest is that member
     is `NotFound` through it, and members that each wait first for another of the cycle
     are `InvalidData`.
- For an index that does not change while a load is in flight, which failure the load
  reports is therefore the same whichever task finished first and whatever the order of
  the requests: a load waits for every dependency to end before it reports, and a cycle
  is decided as a whole. It is the order `asset-api` states and its conformance suite
  pins for every store (`failures_follow_dependencies_in_id_order`): a failing dependency
  before anything of the load's own, the lowest ID first.
- A mount that resolves while a load is in flight is the exception, because it changes
  what the load waits for. A declares [B], B's named version is one its loader refuses,
  and a pack that renames B to a good version is mounted while B's load is in flight. If
  B's load ends first, A finds B failed and fails with its kind, and the mount's rule 3
  then asks for A again; if the mount resolves first, B's named version is the good one,
  in flight, so A waits for it and is published with no failure recorded. Both orders
  end with A published; only one of them records, and so journals, a failure of A.
- Which loads a store starts beyond the ones a request names is the store's own
  (`crates/asset-api/README.md`, "What the store decides"). This store asks for all of a
  load's dependencies when it accepts the load, so that they load side by side with it
  and with each other; the reference store stops at the first failing one. A dependency
  with a higher ID than a failing one is therefore loaded, or recorded as failed, here
  and left untouched there.
- Dependency chains and cycles are as long as a cook makes them, so every walk over
  dependencies is a loop over a work list and never a recursion per link: accepting a
  load's dependencies (step 1), finding and deciding a cycle (step 6), a mount's rule 3,
  and `resolve`. A chain of 100,000 links is accepted in one call on a worker's stack.
- An exact request asks for its dependencies as any load does (step 1): by newest
  version, as the trait states, never by a version from the caller's table. A client
  that requests every entry of a table by exact version, with the server's packs
  mounted, loads each version once.
- `ErasedLoader::load` is one synchronous call: the version check hashes the whole
  content and the loader decodes it, on the background worker, without a yield inside.
  The store yields before the call and the task ends after it. A load therefore holds
  its worker for as long as that one call takes, which for a large asset is longer than
  architecture section 5.1's 0.5 ms slice (`docs/verification.md` has the row; D136).
- `LoadPriority` orders nothing at milestone 1. The store applies no budget: every
  accepted load is spawned at once, none waits behind another in the store, and `io`'s
  queue is first-polled first. The priority is recorded on the load's span; a dependency
  is loaded at its dependant's. A caller that needs one set before another requests it
  first and waits for it.
- Every load task runs under `AllocTag::Asset`, whatever tag the requesting thread held:
  what a loader allocates, the asset included, is charged to `Asset`. The read's buffers
  are `Io`'s, as `crates/io/README.md` states.
- A loader that panics aborts the process, as any task does. A loader returns
  `InvalidData` for bytes it cannot decode (`crates/asset-api/README.md`).

### Publication, current versions and the generation

- Publishing a version is one step under the store's lock: the version becomes `Ready`
  for `get`; it becomes its asset's current version if the index names it at that
  moment, whoever asked for the load and whether or not the asset is followed; the
  generation moves. A version published while the index names another is published for
  `get` alone.
- A failed load of the named version leaves the current version in place: `newest` and
  `resolve` keep answering with it.
- `generation()` moves, by at least one and in the same step as the change, when a
  version is published, when an already published version becomes current, and when a
  failure is recorded. A retry that fails again records its failure again, and so does a
  repeated `request_newest` of an asset no mounted pack holds. It never moves otherwise:
  not for a lookup, not for the accepting of a load, not for a request that does
  nothing, not for a mount's renaming.
- **While the generation is unchanged, `get`, `newest` and `resolve` answer as they
  did.** Accepting a load, a load in flight and a mount's renaming change no answer; an
  answer changes only when a load ends (inside the request, for one that ends there) or
  a published version becomes current, and the generation moves in the same step. In
  particular a recorded failure stays the answer while its retry is in flight and
  is replaced by the retry's outcome, and the newest failure of an asset stays the answer
  after a mount named a new version for it, until the load of that version ends. So a
  caller acts, waits for the generation to move, and looks again; it need not look
  before. This is `asset-api`'s contract as D136 amended it, and its conformance suite
  holds every store to it (`answers_change_only_with_the_generation`). Before, the trait
  read as if a retry or a renaming turned `Failed` into `Pending`, which no store whose
  loads take time can do and keep the generation's promise.
- A lookup never sees a change the generation does not show yet, nor the reverse: the
  change and the move are one step under the store's lock, and `generation()` reads
  under it. A caller that reads the same generation twice has missed nothing between
  the two reads.
- Every end of a retry moves the generation, so the protocol above cannot hang. For the
  same reason a caller retries a failure on a decision (a pack was mounted by hand, a
  player asked), never on every move of the generation: a retry that fails again moves
  it again.

### Hot reload

- Hot reload is the mount rule above and nothing else: when a mount resolves, the store
  asks again, as `request_newest` does, for the followed assets the mount renamed, for
  those whose failure was of the source and to which the mount brings a new source, and
  for the followed assets that had failed for one of either, transitively. That covers
  a new version of a loaded asset, a fixed version of one that failed, an intact copy of
  one whose pack was damaged or unreadable, an asset no pack held when it was first
  asked for, and the dependants any of these had failed: the pack that repairs a
  dependency repairs its dependants at its mount.
- A mount retries nothing it could not have repaired. A followed asset whose named
  version failed with a failure of the version and which the mount does not rename (a
  loader refusing intact bytes, a cooked format the loader no longer reads, a kind
  without a loader) is not read again; neither is one whose failure was of the source
  when the mount does not hold that version; neither is one that failed for a dependency
  the mount does not ask for. A caller's `request_newest` retries any of them, at any
  time.
- The store reloads what it follows and nothing else: an asset loaded only by exact
  version is never reloaded, and nothing is loaded that was not asked for by name, by
  exact version or as a dependency of such a load.
- A reload publishes like any load. A holder of an `AssetHandle` or a `PinnedTable`
  never sees it: a handle is one immutable version. Code outside the boundary sees it by
  asking `newest` again; a region sees it when its driver resolves again, pins the new
  table and enters it through `TickInput::assets`, which is where it is tick-stamped and
  journaled (architecture sections 8.2 and 9.3). The store stamps and journals nothing.
- A load in flight when a mount renames its asset runs to its end and is published for
  `get`; it does not become current, since the index no longer names it, and the load of
  the newly named version decides the asset's current version.
- At milestone 1 someone calls `mount`: the dev loop, a test, a tool. The store reads no
  directory and watches nothing ("Not in this crate yet").

### Lookups

- `get`, `newest`, `resolve` and `generation` never start a load, never wait and never
  block on IO, a decode or a loader: the store holds its lock for its own map work only.
  They are for code between runs, IO completion and presentation; per-entity code inside
  the boundary reads through a `PinnedTable`, which takes no lock and touches no atomic
  (architecture section 3, "No locks on the hot path").
- `get(id, version)`: `Ready` if the version is published; else `Failed` with the
  failure recorded when the last load of it ended, if one ended in failure; else
  `Pending`.
- `newest(id)`: `Ready` with the current version if the asset has one; else `Failed`
  with the asset's newest failure if one is recorded; else `Pending`. A newest failure
  is recorded, replacing the one before, when a load ends in failure while the index
  names its version (the load's failure), whoever asked for the load and whether or not
  the asset is followed, and when `request_newest`, by name or for a dependency, finds
  no mounted pack holding the asset (`{ id, version: None, kind: NotFound }`). The end
  of a load of a version the index does not name then is an answer for `get` alone.
- What an exact request can do to `newest` is therefore only what the end of its load
  does. An asset that is not followed is not reloaded, so its `newest` may stand at a
  version, or a failure, of a version the index named earlier, until `request_newest`
  asks.
- `resolve(set)` is one critical section, so its table is consistent: `Failed` with the
  failure of the lowest `AssetId` in `set` whose `newest` is `Failed`; else `Pending` if
  any member's is `Pending`; else `Ready` with the current version of every member and
  of every asset those versions declare as a dependency, transitively. The walk always
  finds a version, since a version is published only once each of its dependencies has a
  current one and a current version is never withdrawn.
- A table `resolve` returned pins: `PinnedTable::pin` over the same store is `Ready` for
  it, then and at any later time, since nothing is unloaded.
- A version stays available through `get` until the last handle to the store is
  dropped, which is longer than `asset-api`'s "as long as any handle to it is alive"
  requires. Handles and pinned tables own their assets and outlive the store.

### Failures

- A failed load is an `AssetFailure { id, version, kind }` naming the asset and the
  version that failed; a dependant that fails for a dependency names itself and carries
  the dependency's kind. The kinds:

  | Kind | When |
  |---|---|
  | `NotFound` | No mounted pack holds that version of the asset (`request`), or the asset at all (`request_newest`, `version: None`) |
  | `Unsupported` | No loader is registered for the kind the pack's index gives the asset, and no dependency of it failed |
  | `VersionMismatch` | The cooked format version in the index is not the loader's (`ErasedLoader::load`) |
  | `InvalidData` | The content does not hash to its version; the stored range is not one zstd frame of the stated length, or runs past the file (`io`); the loader refused the bytes; a dependency cycle among assets without a current version, for the members step 6 says |
  | `ResourceExhausted` | A buffer the allocator refused, or zstd unable to allocate, in the read (`io`); a loader that returned it |
  | `Io` | The read failed in the operating system |
  | `Cancelled` | The pool stopped before the read ran, or the executor shut down before the load ended |
  | any other | Only as the kind a loader returned, or as the kind of a failed dependency |

- Every recorded failure is logged once at `warn`, target `silo`, with the asset, the
  version, the kind, the dependency it came from if it did, and the message of the
  underlying error, which does not travel in the `AssetFailure`.
- Nothing here panics on bytes from disk: a corrupt trailer, index, frame or payload is
  one of the values above.

### Settling, cancellation and shutdown

- `Store::settled()` resolves once every load accepted before the future's first poll
  has ended. It waits for no load accepted later, except as a load accepted before
  waits for one (a dependency that a later mount renamed). A mount in flight is not a
  load: a caller awaits its `Mount`, and the loads the mount accepts are accepted when it
  resolves.
- Every accepted load ends. One whose task the executor dropped at its shutdown, one
  spawned after that shutdown, and one whose read the stopped pool refused end failed
  with `Cancelled`, recorded like any failure. So `settled()` always resolves, also
  after a shutdown.
- `Mount` and `Settled` submit nothing until first polled and are driven by `.await`,
  `Executor::spawn` or `Executor::block_on`. A `Mount` dropped before it resolved mounts
  nothing; the file, if it was opened, is closed. A `Settled` dropped waits for nothing.
- The last handle to the store is the last of: every `Store` clone, the `Arc<dyn ..>`
  views made from them included, and every `Mount` and `Settled` not yet resolved or
  dropped. Load tasks hold none. Dropping the last handle cancels every load in flight:
  each task is dropped at its next suspension point, a read the pool has not started
  never starts, one it has started finishes on the pool thread where its buffer is
  freed, and a loader's call that is running returns and its asset is dropped. Nothing is
  published afterwards. The drop does not wait for any of it and never blocks. Each
  mounted pack's descriptor closes when the last read on it has ended.
- The store owns no thread and has no shutdown. It keeps one `Executor` handle and one
  `Io` handle for its life; `app` drops it before `Io::shutdown` and
  `Executor::shutdown`. Left alive past them it is inert: its lookups still answer, and
  its requests and mounts end in `Cancelled`.
- There is no cancelling one request: a load once accepted runs to its end unless the
  store is dropped ("Not in this crate yet").

### The region baseline

- `RegionBaseline` is an asset kind like any other to the store: kind
  `AssetKind::from_name("region_baseline")`, cooked format 1, cooked bytes
  `Baseline::encode`'s, no declared dependencies. `RegionBaselineLoader` is
  `Baseline::decode`, and refuses declared dependencies with `InvalidData`. The cooked
  format version moves with `lactose-bundle`'s document format version, in one change;
  a baseline in another document format is `VersionMismatch` from the decode either way.
- A baseline's declared name is `RegionBaseline::name(content_hash)`: `region_baseline/`
  followed by its `Baseline::content_hash` as sixteen lower-case hexadecimal digits. Its
  `AssetId` is `RegionBaseline::id(content_hash)`, `AssetId::from_name` of that name, so
  it shares the one namespace of asset names and RICOTTA's collision check covers it
  with the name the cooker gets from `name`, never one it rebuilds. Equal baselines are
  one asset with one version in every store; a re-cooked region's baseline is another
  asset, and the old one stays loadable while a mounted pack holds it.
- `RegionBaseline::lookup(catalog, content_hash)` is `newest_typed` of that ID, checked:
  an asset of another kind under the ID, or a baseline whose content hash is not
  `content_hash`, is `Failed` with `InvalidData`, the ID and the published version. It
  starts no load; the caller asks first with `request_newest(RegionBaseline::id(..), ..)`.
- A baseline declares no dependencies so that it can be read without its region's
  assets: the load of a save validates every section against its baseline, and the diff
  of an unload needs the baseline alone. The region's asset set is `Baseline::assets`,
  which its driver requests by name.
- Every store registers the loader itself; a loader of this kind among
  `Store::new`'s `loaders` is the duplicate `Store::new` refuses.
- A baseline's load is one `ErasedLoader::load` call like any other kind's, and this
  kind is the first to exceed architecture section 5.1's slice: for a region of 100,000
  rows (an 8.4 MB document) the design review measured the version hash at 3.0 ms and
  `Baseline::decode` at 4.3 ms, 7.3 ms for the one call (release, Apple M1). Nothing
  loads a baseline through the store before milestone 3; `docs/verification.md` has the
  row that decides it there.

### Tracing

Everything is at target `silo` (`LOG_TARGET`), the store's codename (architecture section
19). Asset IDs and versions are written as `AssetFailure`'s `Display` writes them.

| Name | Kind, level | Fields at creation | Recorded later |
|---|---|---|---|
| `silo.mount` | span, `INFO` | `path`, `place` | `assets` (entries in the pack), `requested` (loads the mount accepted), `error` |
| `silo.load` | span, `INFO` | `asset`, `version`, `kind`, `priority`, `cause` (`"exact"`, `"newest"`, `"dependency"` or `"mount"`) | `bytes` (the decoded length), `error` |
| `silo.publish` | event, `INFO`, inside the load's span | `asset`, `version`, `current` (whether it became the current version), `generation` | |
| `silo.current` | event, `INFO` | `asset`, `version`, `generation`: an already published version became current, at a `request_newest` or a mount | |
| a recorded failure | event, `WARN`, inside the load's span | `asset`, `version`, `kind`, `dependency`, the message | |

- `silo.mount` is created at the mount's first poll and closed when the future resolves
  or is dropped; its `io.open` span and its two `io.read` spans are created inside it.
- `silo.load` is created inside the request that accepted the load, as a root span, so it
  never holds the requester's span open; it is entered for each poll of the load's task
  and closed when the load ends. The read's `io.read` span is created inside it, so a
  profiler joins a pack read and its decode slices to the asset they serve. A request
  has no span of its own: one that accepts a load creates that load's span, one that
  does nothing emits nothing. A load that ends inside the accepting call (`NotFound`; a
  kind without a loader and with no dependency to wait for) has its span created and
  closed there, with `error` recorded, and with `kind` empty, and `version` too for a
  newest request, when no entry was found.
- `error` is recorded with the kind, for a failure only. Nothing else is logged at
  `INFO` or above.

## Not in this crate yet

- **Residency: unloading, budgets, releasing a region's set, cancelling one request**
  (milestone 3; `asset-api`'s traits are amended then). Until then every published
  version stays in memory until the last handle to the store is dropped, superseded
  versions included, so a dev session that re-cooks one large asset many times holds
  every version of it; an asset once followed is followed for good; and a
  `RegionRequest::Cancel` or `Release` has nothing to call.
- **A bound on loads in flight, and priority.** Every accepted load is spawned at once.
  A load whose read has returned and that waits for a background worker holds its
  stored and decoded buffers, so the peak of `AllocTag::Io` during a large request is
  not bounded by this crate. Milestone 3's budgets bound it and make `LoadPriority`
  bind, with `io`'s IO budget.
- **Publication of a kind the binary has no loader for.** A version of such a kind is
  `Unsupported`, so `resolve` fails for a set whose closure holds one. A server links no
  renderer and so no mesh or texture loader, yet resolves tables that name them: before
  `app-server` resolves a real region's set, a decision is needed between loaders that
  keep nothing and publication from the index alone (milestone 3;
  `docs/verification.md`).
- **Watching a directory** for the cooker's output (milestone 2, with the `io` request
  `crates/io/README.md` names). Until then whoever knows the cooker ran calls `mount`.
- **Which packs a fresh process mounts, and in which order.** The store takes the order
  of the calls. A manifest, or a naming rule under which `Io::read_dir`'s order is cook
  order, is the cooker lane's to define; nothing in the format depends on it.
- **Unmounting, and merging packs.** One descriptor and one index per mount until the
  store is dropped; a long dev session's mount count against the process's descriptor
  limit is a verification row.
- **A streaming pack writer**, for a pack that does not fit in memory (milestone 3, with
  `io`'s appending writer). The index carries explicit offsets so that it needs no new
  format.
- **Slicing the version check and the loader** to architecture section 5.1's 0.5 ms
  ("Requests and loads" and "The region baseline" above). The store owns the region
  baseline's loader, so it can slice that decode itself; slicing the version hash needs
  an `asset-api` entry that skips `ErasedLoader::load`'s own check. The verification row
  decides both at milestone 3, where a baseline is first loaded through the store.
- **Refusing a stale cooked format from the index.** `ErasedLoader` exposes its kind and
  not its format version, so `VersionMismatch` is found only after the asset was read
  and hashed. An accessor in `asset-api` would let the store refuse it without a read,
  as it refuses a kind without a loader (noted with D136; not needed at milestone 1).
- **Which baseline a region currently has**: the cooked region layout that maps a region
  to its baseline's content hash, its seed and its place in the atlas (milestone 3, with
  streaming). `RegionBaseline` answers only "the baseline with this hash".
- **The hash of simulation-relevant cooked data** for the handshake (architecture
  section 9.1) and **the distribution of a table to clients**: `net` and `app` at
  milestone 4, from the tables the store resolves.
- **A platform's own compression** as a third storage code, with `io`'s `Encoding`
  variant, when a console backend brings a decoder; it is a new pack format version.
- **Loaders registered after the store is built**, and **a pack mounted from memory**:
  no consumer needs either; tests write packs to a temporary directory.

## Proof for milestone 1

The crate passes `cargo fmt`, clippy with `-D warnings`, `cargo xtask tiers`, `cargo xtask
boundary`, `cargo xtask features` and its doctests. `lib.rs` pins at compile time that
`Store` is `Clone + Send + Sync + 'static`, that a shared store coerces to `Arc<dyn
AssetCatalog>` and that to `Arc<dyn PublishedAssets>`, that `Mount` and `Settled` are `Send
+ Unpin + 'static` futures of their documented outputs, that `PackBuilder` is `Send + Sync`
and `Storage` is `Copy`, and that `RegionBaseline` is an `Asset` its `Copy` loader
produces. `tests/sketch.rs` compiles the consumers' code against the surface and pins the
log target, the format version and the baseline kind's key.

Each row below is proved by nextest tests asserting on counts, orders, bytes and values,
never on timing: a bounded wait (60 s) only turns a hang into a failure, and a test loader
held and released by the test (`tests/common`, `Gate`) stands in for a slow one. None runs
under SOUR: this crate is outside the determinism boundary. Tests write packs into
`CARGO_TARGET_TMPDIR` through `Io::replace` and read them through the real pool; the
sparse files of the format and mount-cost rows, and the damage done to a mounted file in
place, are written with `std::fs`. `tests/common` also holds a pack writer written from
"The pack format" rather than from this crate, so that a test can break one rule at a
time, and a capturing `tracing` subscriber, hand-written as `io`'s span tests write
theirs. The file of each row is named at its end.

| What | How |
|---|---|
| Conformance | `asset-api`'s suite, its 21 checks, passes against the store, once over raw packs and once over `Storage::Zstd` packs, each `provide` one pack written and mounted (`tests/conformance.rs`). Among them, for every store: no answer changes while the generation stands still around a renaming cook; a retry that fails again moves the generation; an exact request of a published version changes nothing; the end of a load of the named version answers `newest` whoever asked; the empty ID; and the failure a load reports with several failing dependencies and with a cycle, in both orders of requests |
| A pack read through `io` | Assets of several sizes, empty and several MiB among them, added raw and compressed, written, mounted, requested, and read back equal through `get_typed`; the compressed ones through `read_encoded_at` (the `io.read` span says `zstd`) with the decode on the background worker; a frame no shorter than its bytes is stored raw (`tests/reads.rs`, `tests/pack.rs`) |
| The format | Golden bytes of an empty pack, a one-entry and a two-entry pack; the index sorted whatever the add order; each rule of "The pack format" broken in turn refuses the mount with `InvalidData`, nothing mounted; another format version is `VersionMismatch`, and a file without the magic `InvalidData`, before anything else is read; an index length past the file is `InvalidData` with no read of an index, below 1 GiB and above it (a trailer naming 2 GiB in a file of a few bytes), and one within the file and over 1 GiB is `ResourceExhausted` (a sparse file); neither issues a read of the index, and a count past the index (`u32::MAX` in an index of four bytes) is refused with `InvalidData` (that nothing is reserved first is the decoder's order of checks: an allocator that records block sizes needs `unsafe`, which this crate's tests do not have); `add` refuses `AssetId::NONE`, an ID added again with another version, a level of 0 and of 23 and bytes over the cap, adding nothing, and an ID added again with the same version changes no byte of the pack (`tests/pack.rs`) |
| Hash verification failure as a value | A pack whose index claims a version its content does not hash to (the harness's `claim`), and a pack with one payload byte flipped, raw and compressed: `Failed` with `InvalidData`, the claimed version in the failure, `warn` logged, no panic; the other assets of the pack load (`tests/reads.rs`) |
| Mount order | Three packs holding one asset at three versions: mounted in turn, and with the three futures created in order and awaited in reverse, the named version is the last call's; an exact request finds each of the three; a failed and a dropped mount leave a place empty and change nothing; a path mounted, replaced by rename and mounted again serves both contents (`tests/mounts.rs`) |
| Mount cost | A mount of a pack of 100,000 entries and several hundred MiB of payload issues one `io.open` span and two `io.read` spans, of 24 bytes and of the index's length, and nothing else; its time is recorded with the load averages for the verification row (`tests/spans.rs`) |
| A publish observed through `generation` | With an asset followed and current, a new pack holding a new version is mounted: the generation is unchanged when the mount resolves, moves when the reload publishes, `newest` then answers the new version and `resolve` a new table, while a handle and a `PinnedTable` taken before still read the old one; a pack that renames nothing followed moves nothing; a pack naming an earlier, published version makes it current at the mount and moves the generation (`tests/mounts.rs`) |
| Following | An asset requested only by exact version is not reloaded by a mount; a dependency nobody named is reloaded; an asset requested before any pack held it loads when a pack that holds it is mounted; a missing dependency added by a later pack repairs its dependant, and that dependant's dependant, at that mount; an asset that failed for itself is not read again by the mount of a pack that does not hold it (no `io.read` span, the generation unmoved), and is by `request_newest`; one that failed for a dependency is not retried by a mount that renames neither (`tests/mounts.rs`) |
| A new source for a failed version | An asset and its dependency in one pack, the dependency's payload damaged in place (and, separately, its stored range made to run past the file): both fail. A second pack holding the same versions intact is mounted, renaming nothing: the dependency is read from the second pack and published, and its dependant with it (rules 2 and 3). The same second pack mounted over an asset whose loader refuses its intact bytes, or whose cooked format the loader does not read, or whose kind has no loader, reads nothing and moves nothing; so does a second pack given an earlier place than the damaged one (`tests/mounts.rs`) |
| The generation's promise | Around a retry held in flight by a gated loader, and around a mount that renames a failed asset: `get`, `newest` and `resolve` answer as before until the load ends, and the generation moves exactly when they change; a repeated `request_newest` of an unknown asset moves it each time; requests for what is published or loading never move it; readers on several threads that read the generation, look, and read it again never see two different answers between two equal reads while packs are mounted and loads end under them (`tests/generation.rs`) |
| Failures | Each row of the failure table produced: `Cancelled` by a request after `Io::shutdown`, by a request after `Executor::shutdown` (the request returns, under a bounded wait: the future dropped inside `spawn` records its failure and nothing deadlocks), and for the loads queued behind a gated loader on a one-background-worker executor when it shuts down; a kind without a loader costs no read (no `io.read` span); the reported kind with two failing dependencies is the lower ID's on every run, also when the higher one fails first (gated loaders); a corrupt asset, and one of a kind without a loader, each with a failing dependency, report the dependency's kind (`tests/failures.rs`) |
| Cycles | A declares [B, C] with B below C, no pack holds B, and C declares [A]: with A's loader gated to end first, and with C's gated to end first, A is `NotFound` and C is `NotFound` in both runs; with the cycle's dependency the lower ID, both are `InvalidData` in both runs; neither member's failure is recorded before the other member's loader has ended (once the request has returned, the generation does not move while either loader is held); a load outside the cycle that depends on a member fails with that member's kind (`tests/failures.rs`) |
| Long chains | One pack of a 100,000-link dependency chain: `request_newest` of its head from a task on a background worker accepts every link in that one call, all are published and `resolve` of the head returns 100,000 entries; the same chain closed into a cycle fails every member `InvalidData`; the chain with its last link's dependency missing fails every link `NotFound`, and a pack that adds the missing asset repairs all 100,000 at its mount (rule 3). None overflows a worker's stack (`tests/failures.rs`) |
| Settling | `settled()` resolves only after a gated load ends, resolves while a load accepted after its first poll is still held, covers the dependencies and the loads a mount accepted, and resolves after an executor shutdown (`tests/settling.rs`) |
| Cancellation by drop | On an executor with one background worker, with more loads accepted than that and every loader gated, the last handle is dropped while the first loader's call is held: the drop returns at once; when the gate opens the held call returns, its asset is dropped, and no other loader call ever begins (counted by the loader); `AllocTag::Io` live bytes return, once the pool is idle, to what they were before the store was made (with one pool thread, after one pack was written: what the pool keeps for itself until it stops is not the store's); a handle taken before the drop still reads its asset. On a store kept alive, a `Mount` polled once and dropped mounts nothing: a request for its asset is `NotFound` (`tests/drop.rs`, and `tests/mounts.rs` for a dropped mount) |
| Memory tags and threads | A loader observes `AllocTag::current() == Asset` and a background worker's thread, whatever tag and thread the requester had; the store's lock is not held across a gated loader (a lookup returns while the loader is held) (`tests/settling.rs`) |
| The region baseline | `name` and `id` golden values, `id` the `AssetId::from_name` of `name`; `cook` then a load through the store and through the reference store gives a baseline equal to the original, shared by `Arc`; `lookup` is `Pending` before the request, `Ready` after, `InvalidData` for another kind under the ID and for a baseline under the wrong ID; a baseline cooked with a dependency is refused by the loader (`tests/baseline.rs`) |
| A boundary world's read | With `lactose-ecs` and `lactose-sim` as dev-dependencies: a table resolved and pinned from the store enters a world (built by `SimulationBuilder`, never stepped) through `World::set_region_assets`, the world reads a `SimulationAsset` through the table it holds, and after a reload it reads the old version until the new table is set (`tests/baseline.rs`). A kernel reading it through `Ctx::assets()` needs a world that runs a system, which is a simulation test and so `[harness]` work: that part of the `docs/verification.md` item on `Ctx::assets` stays open |
| Tracing | A capturing subscriber sees the spans and events of the table with their fields, `silo.load` a root span with the read's `io.read` inside it, nothing for a request that does nothing, and one `warn` per recorded failure (`tests/spans.rs`; the `warn` of a damaged asset also in `tests/reads.rs`) |
