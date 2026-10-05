# lactose-asset-api

The asset interface: what every crate that defines, cooks, loads or reads an asset agrees on.
The store behind it is SILO (`lactose-asset`).

| | |
|---|---|
| Tier | foundation |
| Determinism boundary | outside; boundary crates may use its types (architecture section 8.2) |
| Unsafe | none, `#![forbid(unsafe_code)]`. `AssetRef<T>` is plain data through `lactose_core::typed_plain_data!`, whose `unsafe impl`s are justified in `crates/core/SAFETY.md` |

## What it holds

| Item | Purpose |
|---|---|
| `AssetId` | Stable identity: xxh64 of the declared asset name. Zero is `AssetId::NONE` |
| `AssetRef<T>` | The typed reference field components hold. An `AssetId` in memory |
| `Asset`, `AssetKind` | An asset type, its stable kind key and its cooked format version |
| `AssetVersion` | One published version of an asset, assigned by the cooker |
| `VersionTable` | The versions one region sees. Sorted, comparable, hashable |
| `AssetSet` | The assets a region declares. Sorted. `lactose-bundle` embeds it in a baseline |
| `AssetHandle<T>`, `UntypedHandle` | Shared ownership of one published version |
| `PinnedTable` | A version table plus a handle to everything in it: how boundary code reads assets |
| `Processor`, `Loader`, `ErasedLoader` | Source to cooked bytes, and cooked bytes to a runtime asset |
| `CookedAsset`, `CookedHeader` | A processor's output, and the part of it a pack index records |
| `AssetStore` | The `dyn` resource `app` inserts: request, exact lookup, newest lookup, set readiness |
| `Readiness<T>`, `AssetFailure`, `LoadRequest` | The values that cross the store |
| `LoadPriority`, `MemoryBudget`, `IoBudget`, `StreamingBudget` | Units for the streaming system |
| `conformance` (feature) | The suite every store must pass, and `ReferenceStore` |

## Rules a caller should know

**Identity.** An `AssetId` is the hash of a name, like `ComponentKey`. RICOTTA checks every
name it publishes for collisions, in the same pass that builds the pack index, and treats zero
as taken. Kind names are checked where loaders are registered.

**Versions.** The cooker assigns `AssetVersion`s per asset, strictly increasing, and bumps
one when the cooked bytes or the dependency list change. Versions come from cooked data, never
from a process-local counter, so the server, every client and a replay mean the same thing by
`(AssetId, AssetVersion)`. A published version is immutable.

**Dependencies are by `AssetId`, not by version.** A loaded asset keeps `AssetRef`s to its
dependencies and the reader resolves them through its own table. A loader is therefore a
plain function over decompressed bytes and never receives another asset.

**Handle policy.**

| Holder | Holds | Sees |
|---|---|---|
| A component | `AssetRef<T>` | nothing by itself; it is an ID |
| A region | `VersionTable`, and the `PinnedTable` pinned from it | exactly the table's versions, until the table is replaced at the input sync point |
| A system inside the boundary | `&T` borrowed from its region's `PinnedTable` | the pinned version; it has no way to name "newest" |
| Code outside the boundary | `AssetHandle<T>` from `AssetStore::newest_typed` | the version that was newest when it asked; it asks again to see a reload |

- `AssetHandle<T>` is the loaded-asset handle. It is `Arc`-backed: the version stays in memory
  while any handle or pinned table refers to it and is freed after the last one goes, unless
  the store still retains it as the newest.
- `lactose_core::Handle<T>` is not used for assets. It is an index that needs a borrow of the
  table that issued it, and a `dyn` store shared between threads cannot lend `&T` out of a
  table it is concurrently publishing into. Shared ownership of immutable data has no such
  hazard. A store is free to use `HandleTable` internally.
- An `AssetHandle` holds a pointer, so it cannot be a component field. The compiler enforces
  that half of the policy.
- `AssetStore::newest` and `AssetStore::resolve` depend on load timing. Inside the boundary
  only `sim` calls `resolve`, at the input sync point, and the result is journaled as an
  input. Nothing in the type system stops a boundary crate calling `newest`; the boundary
  lint in `xtask` should deny it.

**Loading.** Lookups never start a load and never block. `AssetStore::request` asks for a
load; the answer to a later lookup is `Ready`, `Pending` or `Failed`.

**Region readiness.** On the server, `sim` requests a region's `AssetSet`, polls
`AssetStore::resolve` at the input sync point, and on `Ready` pins the table and lets the
region enter `Simulated`. A client or a replay is given the table, requests each entry by
exact version and pins it with `PinnedTable::pin`.

**Failure.** A missing or failed asset is an `AssetFailure { id, version, kind }`, plain
comparable data that reuses `lactose_core::ErrorKind`. `sim` stamps it with a tick to make
the event of architecture section 14. It converts into `lactose_core::Error`.

## Conformance

A backend enables the `conformance` feature from its dev-dependencies, implements
`conformance::Harness` around its store and calls `conformance::run`. `ReferenceStore` is the
in-memory store the suite is tested against. It is also a stand-in for tests of crates that
need a store. It is not a backend: no IO, no unloading, no budgets, one lock.
